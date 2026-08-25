<script setup lang="ts">
/**
 * 文件类型图形：单一线条风格，1.5px 描边，随 currentColor。
 * 不用 emoji、不用彩色图标块——层级交给排版。
 */
import { computed } from "vue";
import type { ItemKind } from "@/domain/types";

const props = withDefaults(
  defineProps<{ kind: ItemKind; ext?: string | null; size?: number }>(),
  { ext: null, size: 20 },
);

type Glyph =
  | "image"
  | "video"
  | "audio"
  | "doc"
  | "sheet"
  | "slides"
  | "pdf"
  | "code"
  | "archive"
  | "text"
  | "folder"
  | "shortcut"
  | "file";

const glyph = computed<Glyph>(() => {
  if (props.kind === "folder") return "folder";
  if (props.kind === "shortcut") return "shortcut";
  if (props.kind === "text") return "text";
  const e = (props.ext ?? "").toLowerCase();
  if (["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "heic"].includes(e))
    return "image";
  if (["mp4", "mkv", "mov", "avi", "webm", "wmv", "flv"].includes(e)) return "video";
  if (["mp3", "wav", "flac", "aac", "ogg", "m4a", "wma"].includes(e)) return "audio";
  if (["doc", "docx", "rtf", "odt", "pages"].includes(e)) return "doc";
  if (["xls", "xlsx", "csv", "ods", "numbers"].includes(e)) return "sheet";
  if (["ppt", "pptx", "odp", "key"].includes(e)) return "slides";
  if (e === "pdf" || e === "md" || e === "markdown") return "pdf";
  if (
    [
      "ts", "tsx", "js", "jsx", "vue", "rs", "py", "java", "c", "h", "cpp", "cs", "go",
      "rb", "php", "swift", "kt", "sh", "bat", "ps1", "html", "css", "scss", "json", "yaml", "yml", "toml",
    ].includes(e)
  )
    return "code";
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"].includes(e)) return "archive";
  if (["txt", "log", "ini", "conf"].includes(e)) return "text";
  return "file";
});
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <template v-if="glyph === 'folder'">
      <path d="M3.5 7.5a1.5 1.5 0 0 1 1.5-1.5h4l2 2.5h8a1.5 1.5 0 0 1 1.5 1.5v7a1.5 1.5 0 0 1-1.5 1.5H5a1.5 1.5 0 0 1-1.5-1.5Z" />
    </template>
    <template v-else-if="glyph === 'shortcut'">
      <path d="M10 14 20 4" />
      <path d="M14 4h6v6" />
      <path d="M11 5H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-5" />
    </template>
    <template v-else-if="glyph === 'text'">
      <path d="M5 4.5A1.5 1.5 0 0 1 6.5 3h7L19 8.5v11a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5Z" />
      <path d="M13 3v6h6" />
      <path d="M8.5 13h7M8.5 16.5h5" />
    </template>
    <template v-else-if="glyph === 'image'">
      <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
      <circle cx="9" cy="10" r="1.6" />
      <path d="m3.5 17 4.8-4.5 3.4 3.2 3.6-3.8 5.2 5" />
    </template>
    <template v-else-if="glyph === 'video'">
      <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
      <path d="m10 9.2 5 2.8-5 2.8Z" />
    </template>
    <template v-else-if="glyph === 'audio'">
      <path d="M9 17.5V6.8l9-1.8v10.7" />
      <circle cx="6.8" cy="17.7" r="2.2" />
      <circle cx="15.8" cy="15.9" r="2.2" />
    </template>
    <template v-else-if="glyph === 'doc' || glyph === 'pdf'">
      <path d="M5 4.5A1.5 1.5 0 0 1 6.5 3h7L19 8.5v11a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5Z" />
      <path d="M13 3v6h6" />
      <path d="M8.5 16.5h7M8.5 13h7M8.5 9.5h2" />
    </template>
    <template v-else-if="glyph === 'sheet'">
      <path d="M5 4.5A1.5 1.5 0 0 1 6.5 3h7L19 8.5v11a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5Z" />
      <path d="M13 3v6h6" />
      <path d="M8.5 12.5h7M8.5 15.5h7" />
    </template>
    <template v-else-if="glyph === 'slides'">
      <path d="M5 4.5A1.5 1.5 0 0 1 6.5 3h7L19 8.5v11a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5Z" />
      <path d="M13 3v6h6" />
      <path d="M8.5 12.5 11 15l3.5-4" />
    </template>
    <template v-else-if="glyph === 'code'">
      <path d="m8.5 8.5-4 3.5 4 3.5M15.5 8.5l4 3.5-4 3.5M13.5 5.5l-3 13" />
    </template>
    <template v-else-if="glyph === 'archive'">
      <rect x="4.5" y="4.5" width="15" height="5" rx="1.5" />
      <path d="M6 9.5v9a1.5 1.5 0 0 0 1.5 1.5h9a1.5 1.5 0 0 0 1.5-1.5v-9" />
      <path d="M10.5 13h3" />
    </template>
    <template v-else>
      <path d="M5 4.5A1.5 1.5 0 0 1 6.5 3h7L19 8.5v11a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5Z" />
      <path d="M13 3v6h6" />
    </template>
  </svg>
</template>
