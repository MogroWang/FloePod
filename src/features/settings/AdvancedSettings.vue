<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useSettingsEditor } from "./context";
import AutoBlockSettings from "./AutoBlockSettings.vue";
import PodRulesEditor from "@/components/PodRulesEditor.vue";
import PodSecurityEditor from "@/components/PodSecurityEditor.vue";
import type { Pod, PodRules, PodSecurity } from "@/domain/types";
const { s, savePod } = useSettingsEditor();

/* 规则匣与敏感匣是两个独立功能，各自选择匣、各自编辑，互不影响。 */
const rulesPodId = ref<number | null>(null);
const securityPodId = ref<number | null>(null);

const firstPodId = computed(() => s.value.pods[0]?.id ?? null);
watch(
  () => s.value.pods.map((pod) => pod.id).join(","),
  () => {
    // 当前选中匣被删除时回落到第一个匣。
    if (rulesPodId.value && !s.value.pods.some((pod) => pod.id === rulesPodId.value)) {
      rulesPodId.value = firstPodId.value;
    }
    if (securityPodId.value && !s.value.pods.some((pod) => pod.id === securityPodId.value)) {
      securityPodId.value = firstPodId.value;
    }
  },
  { immediate: true },
);

const rulesPod = computed(() => s.value.pods.find((pod) => pod.id === rulesPodId.value) ?? null);
const securityPod = computed(
  () => s.value.pods.find((pod) => pod.id === securityPodId.value) ?? null,
);

/* 回调参数的空值由这里统一判断，模板保持简洁。 */
function saveRules(pod: Pod | null, rules: PodRules) {
  if (!pod) return;
  void savePod(pod.id, { rules });
}
function saveSecurity(pod: Pod | null, security: PodSecurity) {
  if (!pod) return;
  void savePod(pod.id, { security });
}
</script>
<template>
  <div>
    <h2 class="page-title">高级设置</h2>
    <p class="page-desc">自动屏蔽，以及每个匣的规则匣、敏感匣。</p>

    <AutoBlockSettings />

    <h3 class="section-title adv-pod-section">规则匣</h3>
    <p class="group-hint adv-section-hint">
      暂存时按类型、名称、来源、大小过滤，可自动命名与归档。
    </p>
    <div class="pod-picker" role="tablist" aria-label="选择要设置规则的匣">
      <button
        v-for="pod in s.pods"
        :key="pod.id"
        type="button"
        role="tab"
        class="pod-chip"
        :class="{ active: rulesPod?.id === pod.id, off: !pod.enabled }"
        :aria-selected="rulesPod?.id === pod.id"
        @click="rulesPodId = pod.id"
      >
        {{ pod.name }}
        <span v-if="!pod.enabled" class="chip-badge">已停用</span>
      </button>
    </div>
    <div v-if="rulesPod" class="pod-card" :class="{ off: !rulesPod.enabled }">
      <PodRulesEditor :rules="rulesPod.rules" @update="(rules) => saveRules(rulesPod, rules)" />
    </div>

    <h3 class="section-title adv-pod-section">敏感匣</h3>
    <p class="group-hint adv-section-hint">
      加密与解锁交给 Windows（EFS + Hello），应用不保存密码。
    </p>
    <div class="pod-picker" role="tablist" aria-label="选择要设置敏感保护的匣">
      <button
        v-for="pod in s.pods"
        :key="pod.id"
        type="button"
        role="tab"
        class="pod-chip"
        :class="{ active: securityPod?.id === pod.id, off: !pod.enabled }"
        :aria-selected="securityPod?.id === pod.id"
        @click="securityPodId = pod.id"
      >
        {{ pod.name }}
        <span v-if="!pod.enabled" class="chip-badge">已停用</span>
      </button>
    </div>
    <div v-if="securityPod" class="pod-card" :class="{ off: !securityPod.enabled }">
      <PodSecurityEditor
        :pod-id="securityPod.id"
        :folder="securityPod.stagingFolder"
        :security="securityPod.security"
        @update="(security) => saveSecurity(securityPod, security)"
      />
    </div>
  </div>
</template>
