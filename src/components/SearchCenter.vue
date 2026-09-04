<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";
import type { SearchHit } from "@/domain/types";
import { ipc } from "@/ipc/client";

const query = ref("");
const hits = ref<SearchHit[]>([]);
const loading = ref(false);
const indexing = ref(false);
const message = ref("");
const editing = ref<SearchHit | null>(null);
const tagsDraft = ref("");
const noteDraft = ref("");
let searchTimer: number | undefined;

async function runSearch() {
  window.clearTimeout(searchTimer);
  loading.value = true;
  try {
    hits.value = await ipc.searchItems(query.value);
    message.value = `找到 ${hits.value.length} 项`;
  } catch (error) {
    message.value = `搜索失败：${String(error)}`;
  } finally {
    loading.value = false;
  }
}

function queueSearch() {
  window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(runSearch, 180);
}

async function rebuild() {
  if (indexing.value) return;
  indexing.value = true;
  message.value = "正在本机提取文档文字和图片 OCR，不会上传内容…";
  try {
    const result = await ipc.rebuildSearchIndex();
    message.value = `索引完成：${result.indexed} 项已索引，${result.skipped} 项跳过，${result.failures.length} 项需要检查。`;
    await runSearch();
  } catch (error) {
    message.value = `索引失败：${String(error)}`;
  } finally {
    indexing.value = false;
  }
}

function edit(hit: SearchHit) {
  editing.value = hit;
  tagsDraft.value = hit.tags.join(", ");
  noteDraft.value = hit.note;
}

async function saveAnnotation() {
  if (!editing.value) return;
  const tags = tagsDraft.value
    .split(/[，,;；]+/)
    .map((tag) => tag.trim())
    .filter(Boolean);
  try {
    await ipc.updateItemAnnotation(editing.value.item.id, tags, noteDraft.value);
    editing.value = null;
    message.value = "标签和备注已保存到本地数据库。";
    await runSearch();
  } catch (error) {
    message.value = `保存失败：${String(error)}`;
  }
}

onBeforeUnmount(() => window.clearTimeout(searchTimer));
</script>

<template>
  <div class="search-center">
    <div class="search-bar">
      <label>
        <span class="visually-hidden">搜索暂存文件</span>
        <input
          v-model="query"
          type="search"
          placeholder="搜索文件名、OCR、正文、标签、备注或来源…"
          @input="queueSearch"
          @keydown.enter.prevent="runSearch"
        />
      </label>
      <button type="button" :disabled="indexing" @click="rebuild">
        {{ indexing ? "正在建立索引…" : "更新本地索引" }}
      </button>
    </div>
    <p class="syntax">筛选示例：<code>tag:报销</code>、<code>type:pdf</code>、<code>来源:Downloads</code>、<code>after:2026-09-01</code>、<code>上周</code></p>
    <p class="message" role="status" aria-live="polite">{{ message }}</p>

    <div v-if="loading" class="empty">正在搜索…</div>
    <div v-else-if="!hits.length && query" class="empty">没有匹配项目。可先点“更新本地索引”。</div>
    <ul v-else class="results" aria-label="本地搜索结果">
      <li v-for="hit in hits" :key="hit.item.id">
        <div class="result-main">
          <strong>{{ hit.item.name }}</strong>
          <span>{{ hit.matchedOn.join("、") || "路径/筛选条件" }}</span>
          <p v-if="hit.snippet">{{ hit.snippet }}</p>
          <div v-if="hit.tags.length" class="tags">
            <span v-for="tag in hit.tags" :key="tag">#{{ tag }}</span>
          </div>
          <small v-if="hit.note">{{ hit.note }}</small>
        </div>
        <div class="result-actions">
          <button type="button" @click="ipc.openStagedItem(hit.item.id)">打开</button>
          <button type="button" @click="ipc.revealStagedItems([hit.item.id])">位置</button>
          <button type="button" @click="edit(hit)">标签/备注</button>
        </div>
      </li>
    </ul>

    <div v-if="editing" class="annotation" role="dialog" aria-modal="true" aria-labelledby="annotation-title">
      <h3 id="annotation-title">{{ editing.item.name }}</h3>
      <label>
        <span>标签（逗号分隔）</span>
        <input v-model="tagsDraft" maxlength="600" />
      </label>
      <label>
        <span>备注</span>
        <textarea v-model="noteDraft" rows="4" maxlength="2000" />
      </label>
      <div>
        <button type="button" class="primary" @click="saveAnnotation">保存</button>
        <button type="button" @click="editing = null">取消</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.search-center {
  display: grid;
  gap: 9px;
}
.search-bar {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 8px;
}
.search-bar label,
.search-bar input {
  width: 100%;
}
input,
textarea {
  min-height: 38px;
  padding: 7px 10px;
  border: 1px solid var(--line-strong);
  border-radius: 9px;
  background: var(--surface-raised);
  color: var(--ink);
}
textarea {
  resize: vertical;
}
button {
  min-height: 36px;
  padding: 0 11px;
  border: 1px solid var(--line-strong);
  border-radius: 8px;
  background: var(--surface-raised);
  color: var(--ink);
}
button.primary {
  border-color: var(--accent);
  background: var(--accent);
  color: var(--on-accent);
}
.syntax,
.message {
  margin: 0;
  color: var(--ink-2);
  font-size: 11.5px;
}
.syntax code {
  padding: 1px 4px;
  border-radius: 4px;
  background: var(--surface-2);
}
.message {
  min-height: 18px;
}
.empty {
  padding: 18px;
  border: 1px dashed var(--line-strong);
  border-radius: 10px;
  color: var(--ink-2);
  text-align: center;
}
.results {
  display: grid;
  gap: 7px;
  max-height: 360px;
  margin: 0;
  padding: 0;
  overflow: auto;
  list-style: none;
}
.results li {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
  padding: 10px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--surface-raised);
}
.result-main {
  min-width: 0;
}
.result-main strong,
.result-main small {
  display: block;
  overflow-wrap: anywhere;
}
.result-main > span,
.result-main small {
  color: var(--ink-2);
  font-size: 11px;
}
.result-main p {
  margin: 5px 0;
  color: var(--ink-2);
  font-size: 11.5px;
}
.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin: 4px 0;
}
.tags span {
  padding: 1px 6px;
  border-radius: 999px;
  background: var(--accent-soft);
  color: var(--ink);
  font-size: 10.5px;
}
.result-actions {
  display: flex;
  flex-shrink: 0;
  gap: 5px;
}
.result-actions button {
  min-height: 30px;
  padding: 0 8px;
  font-size: 11px;
}
.annotation {
  display: grid;
  gap: 10px;
  padding: 14px;
  border: 1px solid var(--line-strong);
  border-radius: 12px;
  background: var(--surface-raised);
  box-shadow: var(--shadow-pop);
}
.annotation h3 {
  margin: 0;
  font-size: 14px;
}
.annotation label {
  display: grid;
  gap: 4px;
  color: var(--ink-2);
  font-size: 11.5px;
}
.annotation > div {
  display: flex;
  gap: 7px;
  justify-content: flex-end;
}
.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
}
</style>
