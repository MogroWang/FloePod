<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ask, open } from "@tauri-apps/plugin-dialog";
import type { OperationEntry } from "@/domain/types";
import { ipc } from "@/ipc/client";

const operations = ref<OperationEntry[]>([]);
const hours = ref(24);
const busyId = ref<number | null>(null);
const loading = ref(false);
const verifyBusy = ref(false);
const message = ref("");

const latestUndoable = computed(() => operations.value.find((operation) => operation.undoable));

function operationLabel(kind: string): string {
  return ({
    stage: "暂存文件",
    stage_text: "暂存文字",
    export: "导出文件",
    remove: "移出暂存",
    handoff: "可信交接",
    privacy_export: "安全导出",
  } as Record<string, string>)[kind] ?? kind;
}

function formatTime(value: number): string {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
}

function statusLabel(value: string): string {
  return ({
    completed: "已完成",
    partial: "部分完成",
    failed: "失败",
    undone: "已撤销",
    undo_failed: "撤销未完全成功",
  } as Record<string, string>)[value] ?? value;
}

async function refresh() {
  loading.value = true;
  try {
    operations.value = await ipc.listOperations(hours.value, 200);
  } catch (error) {
    message.value = `读取操作记录失败：${String(error)}`;
  } finally {
    loading.value = false;
  }
}

async function confirmUndo(operation: OperationEntry): Promise<boolean> {
  const text = `撤销“${operation.summary}”？\n\n如果文件在操作后已经变化，FloePod 会保守拒绝删除或覆盖。`;
  if (!ipc.inTauri) return window.confirm(text);
  return ask(text, { title: "确认撤销", kind: "warning" });
}

async function undo(operation: OperationEntry) {
  if (busyId.value !== null || !(await confirmUndo(operation))) return;
  busyId.value = operation.id;
  try {
    const result = await ipc.undoOperation(operation.id);
    message.value = result.failed.length
      ? `已恢复 ${result.restored} 项；${result.failed.length} 项需要人工检查。`
      : `已撤销，恢复 ${result.restored} 项。`;
    await refresh();
  } catch (error) {
    message.value = `撤销失败：${String(error)}`;
  } finally {
    busyId.value = null;
  }
}

async function retry(operation: OperationEntry) {
  if (busyId.value !== null) return;
  busyId.value = operation.id;
  try {
    await ipc.retryOperation(operation.id);
    message.value = "失败项已重新执行，请查看最新一条操作记录。";
    await refresh();
  } catch (error) {
    message.value = `重试失败：${String(error)}`;
  } finally {
    busyId.value = null;
  }
}

async function verifyHandoff() {
  if (verifyBusy.value) return;
  const directory = ipc.inTauri
    ? await open({ directory: true, multiple: false, title: "选择要验证的 FloePod 交接包" })
    : "D:\\交接包";
  if (typeof directory !== "string") return;
  verifyBusy.value = true;
  try {
    const result = await ipc.verifyHandoff(directory);
    message.value = result.issues.length
      ? `验证了 ${result.checked} 个文件：${result.valid} 个一致，${result.issues.length} 个缺失或已变化。`
      : `验证通过：${result.valid}/${result.checked} 个文件内容一致。`;
  } catch (error) {
    message.value = `无法验证交接包：${String(error)}`;
  } finally {
    verifyBusy.value = false;
  }
}

onMounted(refresh);
</script>

<template>
  <div class="safety-center">
    <section v-if="latestUndoable" class="regret-card" aria-labelledby="regret-title">
      <div>
        <h3 id="regret-title">刚才的操作不对？</h3>
        <p>{{ latestUndoable.summary }}</p>
      </div>
      <button
        type="button"
        class="regret-button"
        :disabled="busyId !== null"
        @click="undo(latestUndoable)"
      >
        一键恢复
      </button>
    </section>

    <div class="history-toolbar">
      <label>
        <span>查看范围</span>
        <select v-model.number="hours" @change="refresh">
          <option :value="24">最近 24 小时</option>
          <option :value="720">最近 30 天</option>
          <option :value="2160">最近 90 天</option>
        </select>
      </label>
      <button type="button" class="secondary-button" :disabled="loading" @click="refresh">
        {{ loading ? "正在刷新…" : "刷新" }}
      </button>
      <button type="button" class="secondary-button" :disabled="verifyBusy" @click="verifyHandoff">
        {{ verifyBusy ? "正在验证…" : "验证交接包…" }}
      </button>
    </div>

    <p class="sr-status" role="status" aria-live="polite">{{ message }}</p>

    <div v-if="!loading && operations.length === 0" class="empty-history">
      尚无操作记录。下一次暂存、导出或移出文件后会显示在这里。
    </div>

    <ol v-else class="timeline" aria-label="文件操作时间线">
      <li v-for="operation in operations" :key="operation.id" class="operation-card">
        <div class="operation-marker" aria-hidden="true" />
        <div class="operation-main">
          <div class="operation-head">
            <div>
              <span class="operation-kind">{{ operationLabel(operation.kind) }}</span>
              <time :datetime="new Date(operation.createdAt).toISOString()">
                {{ formatTime(operation.createdAt) }}
              </time>
            </div>
            <span class="operation-status" :data-status="operation.status">
              {{ statusLabel(operation.status) }}
            </span>
          </div>
          <p class="operation-summary">{{ operation.summary }}</p>
          <details v-if="operation.items.length" class="operation-details">
            <summary>查看 {{ operation.items.length }} 个项目</summary>
            <ul>
              <li v-for="item in operation.items" :key="item.id">
                <strong>{{ item.name }}</strong>
                <span>{{ statusLabel(item.status) }}</span>
                <small v-if="item.sourcePath">来源：{{ item.sourcePath }}</small>
                <small v-if="item.targetPath">目标：{{ item.targetPath }}</small>
                <small v-if="item.error" class="item-error">{{ item.error }}</small>
              </li>
            </ul>
          </details>
          <div v-if="operation.undoable || operation.retryable" class="operation-actions">
            <button
              v-if="operation.undoable"
              type="button"
              class="secondary-button"
              :disabled="busyId !== null"
              @click="undo(operation)"
            >
              撤销这次操作
            </button>
            <button
              v-if="operation.retryable"
              type="button"
              class="secondary-button"
              :disabled="busyId !== null"
              @click="retry(operation)"
            >
              只重试失败项
            </button>
          </div>
        </div>
      </li>
    </ol>
  </div>
</template>

<style scoped>
.safety-center {
  display: grid;
  gap: 16px;
}
.regret-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  padding: 16px;
  border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--line));
  border-radius: 14px;
  background: var(--accent-soft);
}
.regret-card h3 {
  margin: 0 0 4px;
  font-size: 16px;
}
.regret-card p {
  margin: 0;
  color: var(--ink-2);
}
.regret-button,
.secondary-button {
  min-height: 36px;
  padding: 0 14px;
  border: 1px solid var(--line-strong);
  border-radius: 10px;
  background: var(--surface-raised);
  color: var(--ink);
  font-weight: 650;
  cursor: pointer;
}
.regret-button {
  min-width: 110px;
  border-color: var(--accent);
  background: var(--accent);
  color: var(--on-accent);
}
button:disabled {
  opacity: 0.55;
  cursor: wait;
}
.history-toolbar {
  display: flex;
  align-items: end;
  justify-content: space-between;
  gap: 12px;
}
.history-toolbar label {
  display: grid;
  gap: 5px;
  color: var(--ink-2);
  font-size: 12px;
}
.history-toolbar select {
  min-height: 36px;
  padding: 0 34px 0 10px;
  border: 1px solid var(--line-strong);
  border-radius: 9px;
  background: var(--surface-raised);
  color: var(--ink);
}
.sr-status {
  min-height: 20px;
  margin: 0;
  color: var(--ink-2);
}
.empty-history {
  padding: 26px;
  border: 1px dashed var(--line-strong);
  border-radius: 12px;
  color: var(--ink-2);
  text-align: center;
}
.timeline {
  display: grid;
  gap: 0;
  margin: 0;
  padding: 0;
  list-style: none;
}
.operation-card {
  position: relative;
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr);
  gap: 10px;
  padding-bottom: 14px;
}
.operation-card:not(:last-child)::before {
  position: absolute;
  left: 7px;
  top: 17px;
  bottom: 0;
  width: 2px;
  background: var(--line);
  content: "";
}
.operation-marker {
  z-index: 1;
  width: 16px;
  height: 16px;
  margin-top: 15px;
  border: 4px solid var(--surface);
  border-radius: 50%;
  background: var(--accent);
}
.operation-main {
  padding: 12px 14px;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface-raised);
}
.operation-head,
.operation-head > div {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.operation-kind {
  font-weight: 700;
}
.operation-head time {
  color: var(--ink-3);
  font-size: 11.5px;
}
.operation-status {
  padding: 2px 7px;
  border-radius: 999px;
  background: var(--surface-2);
  color: var(--ink-2);
  font-size: 11px;
}
.operation-status[data-status="failed"],
.operation-status[data-status="undo_failed"] {
  color: var(--danger);
}
.operation-summary {
  margin: 6px 0 0;
}
.operation-details {
  margin-top: 10px;
  color: var(--ink-2);
}
.operation-details summary {
  cursor: pointer;
}
.operation-details ul {
  display: grid;
  gap: 8px;
  margin: 8px 0 0;
  padding: 0;
  list-style: none;
}
.operation-details li {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 2px 10px;
  padding: 8px;
  border-radius: 8px;
  background: var(--surface);
}
.operation-details small {
  grid-column: 1 / -1;
  overflow-wrap: anywhere;
}
.item-error {
  color: var(--danger);
}
.operation-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 10px;
}
</style>
