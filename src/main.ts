import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import "./styles/main.css";
import { ipc } from "@/ipc/client";

// 全局错误上报：把前端异常写到数据目录 debug.log，便于排查。
// 用 busy 标记防止上报失败时（如能力未授权）自我触发无限循环。
let reportingBusy = false;
function reportFrontend(msg: string) {
  if (!ipc.inTauri || reportingBusy) return;
  reportingBusy = true;
  void ipc
    .logFrontend(msg)
    .catch(() => {
      // 上报失败必须在此消费，否则会再次触发 unhandledrejection 并循环记录。
    })
    .finally(() => {
      reportingBusy = false;
    });
}
window.addEventListener("error", (e) => reportFrontend(`error: ${e.message}`));
window.addEventListener("unhandledrejection", (e) => {
  reportFrontend(`unhandledrejection: ${String(e.reason)}`);
});

// 禁用 WebView 默认右键菜单：所有窗口的右键操作由应用内专属菜单接管
// （文件条目走菜单窗口；输入框等区域不提供菜单，属于预期行为）。
document.addEventListener("contextmenu", (event) => event.preventDefault());

createApp(App).use(createPinia()).mount("#app");
