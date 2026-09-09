//! Read actual Tauri function signatures instead of maintaining a second command list.
use quote::ToTokens;
use std::{env, fs, path::PathBuf};
use syn::{FnArg, GenericArgument, Item, Pat, PathArguments, ReturnType, Type};

fn camel(raw: &str) -> String {
    let mut parts = raw.split('_');
    let mut name = parts.next().unwrap_or_default().to_string();
    for part in parts {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            name.extend(first.to_uppercase());
            name.extend(chars);
        }
    }
    name
}

pub fn generate() {
    println!("cargo:rerun-if-changed=src/commands.rs");
    println!("cargo:rerun-if-changed=contract/build.rs");
    let file = syn::parse_file(&fs::read_to_string("src/commands.rs").unwrap()).unwrap();
    let mut output = String::from(
        "fn command_schemas() -> serde_json::Value { let mut commands = serde_json::Map::new();\n",
    );
    let mut handlers = Vec::new();
    for item in file.items {
        let Item::Fn(function) = item else { continue };
        if !function.attrs.iter().any(|attr| {
            attr.path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "command")
        }) {
            continue;
        }
        let name = function.sig.ident.to_string();
        handlers.push(format!("crate::commands::{name}"));
        output.push_str("{ let mut args = serde_json::Map::new();\n");
        let mut optional = Vec::new();
        for input in function.sig.inputs {
            let FnArg::Typed(argument) = input else {
                panic!("IPC receiver unsupported")
            };
            let Pat::Ident(ident) = *argument.pat else {
                panic!("IPC destructured argument unsupported")
            };
            let ty = argument.ty.to_token_stream().to_string();
            if ty == "AppHandle" {
                continue;
            }
            if matches!(&*argument.ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Option"))
            {
                optional.push(camel(&ident.ident.to_string()));
            }
            output.push_str(&format!(
                "args.insert({:?}.into(), input_schema::<{ty}>());\n",
                camel(&ident.ident.to_string())
            ));
        }
        let mut result: Type = match function.sig.output {
            ReturnType::Default => syn::parse_quote!(()),
            ReturnType::Type(_, ty) => *ty,
        };
        if let Type::Path(path) = &result {
            if let Some(segment) = path.path.segments.last() {
                if segment.ident == "Result" {
                    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                        panic!("invalid Result")
                    };
                    let Some(GenericArgument::Type(ok)) = arguments.args.first() else {
                        panic!("missing result type")
                    };
                    result = ok.clone();
                }
            }
        }
        let result = result.to_token_stream().to_string();
        output.push_str(&format!("commands.insert({name:?}.into(), serde_json::json!({{\"args\": args, \"optionalArgs\": {optional:?}, \"result\": output_schema::<{result}>()}})); }}\n"));
    }
    output.push_str("serde_json::Value::Object(commands) }\n");
    let output = output.replace(
        "{ let mut args = serde_json::Map::new();\ncommands.insert",
        "{ let args = serde_json::Map::new();\ncommands.insert",
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("command-contract.rs"),
        output,
    )
    .unwrap();
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("invoke-handler.rs"),
        format!("tauri::generate_handler![{}]", handlers.join(",")),
    )
    .unwrap();
}
