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
    <p class="page-desc">自动屏蔽，以及每个匣的规则匣、敏感匣等进阶能力。</p>

    <AutoBlockSettings />
    <h3 class="section-title adv-pod-section">规则匣与敏感匣</h3>
    <p class="page-desc">按匣生效，两个能力相互独立；在「匣」页可以新建、重命名或删除匣。</p>
    <div v-if="s.pods.length > 1" class="pod-picker" role="tablist" aria-label="选择要设置的匣">
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
      :key="`rules-${pod.id}`"
      class="pod-card"
      :class="{ off: !pod.enabled }"
    >
      <div class="adv-pod-head">
        <span class="pod-name-text" :title="pod.name">{{ pod.name }}</span>
        <span v-if="!pod.enabled" class="adv-pod-off">已停用，启用后以下设置才会生效</span>
      </div>
      <div class="pod-group">
        <div class="group-title">规则匣</div>
        <p class="group-hint">按文件类型、名称、来源和大小过滤，并自动命名、归档或生成校验值。</p>
        <PodRulesEditor :rules="pod.rules" @update="(rules) => savePod(pod.id, { rules })" />
      </div>
    </div>
    <div
      v-for="pod in selectedPodList"
      :key="`security-${pod.id}`"
      class="pod-card"
      :class="{ off: !pod.enabled }"
    >
      <div class="adv-pod-head">
        <span class="pod-name-text" :title="pod.name">{{ pod.name }}</span>
        <span v-if="!pod.enabled" class="adv-pod-off">已停用，启用后以下设置才会生效</span>
      </div>
      <div class="pod-group">
        <div class="group-title">敏感匣、自动锁定与保留期限</div>
        <p class="group-hint">使用 Windows EFS 与 Windows Hello；不上传内容，也不保存自制密码。</p>
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
