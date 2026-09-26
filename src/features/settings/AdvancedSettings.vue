<script setup lang="ts">
import { useSettingsEditor } from "./context";
import { useSelectedPod } from "./context";
import AutoBlockSettings from "./AutoBlockSettings.vue";
import PodRulesEditor from "@/components/PodRulesEditor.vue";
import PodSecurityEditor from "@/components/PodSecurityEditor.vue";
const { s, savePod } = useSettingsEditor();
const { selectedPod, selectedPodList, selectPod } = useSelectedPod();
</script>
<template>
  <div>
    <h2 class="page-title">高级设置</h2>
    <p class="page-desc">自动屏蔽与每个匣的规则匣、敏感匣。</p>

    <AutoBlockSettings />
    <h3 class="section-title adv-pod-section">规则匣与敏感匣</h3>
    <div class="pod-picker" role="tablist" aria-label="选择要设置的匣">
      <button
        v-for="pod in s.pods"
        :key="pod.id"
        type="button"
        role="tab"
        class="pod-chip"
        :class="{ active: selectedPod?.id === pod.id, off: !pod.enabled }"
        :aria-selected="selectedPod?.id === pod.id"
        @click="selectPod(pod.id)"
      >
        {{ pod.name }}
        <span v-if="!pod.enabled" class="chip-badge">已停用</span>
      </button>
    </div>
    <div
      v-for="pod in selectedPodList"
      :key="pod.id"
      class="pod-card"
      :class="{ off: !pod.enabled }"
    >
      <div class="pod-group">
        <div class="group-title">规则匣</div>
        <p class="group-hint">暂存时按类型、名称、来源、大小过滤，可自动命名与归档。</p>
        <PodRulesEditor :rules="pod.rules" @update="(rules) => savePod(pod.id, { rules })" />
      </div>
      <div class="sep" />
      <div class="pod-group">
        <div class="group-title">敏感匣</div>
        <p class="group-hint">加密与解锁交给 Windows（EFS + Hello），应用不保存密码。</p>
        <PodSecurityEditor
          :pod-id="pod.id"
          :folder="pod.stagingFolder"
          :security="pod.security"
          @update="(security) => savePod(pod.id, { security })"
        />
      </div>
    </div>
  </div>
</template>
