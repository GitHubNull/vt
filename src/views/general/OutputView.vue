<script setup>
/**
 * 通用 › 文本输出（T-013，F-SET-02、PRD 3.8.3）。
 */
import { computed, onMounted } from 'vue'
import ToggleSwitch from 'primevue/toggleswitch'
import Message from 'primevue/message'
import SettingsRow from '../../components/SettingsRow.vue'
import { useConfigStore } from '../../stores/config'
import { t } from '../../i18n'

const store = useConfigStore()

onMounted(() => {
  void store.ensureLoaded()
})

function updateOutput(patch) {
  void store.updateSection('output', { ...store.sections.output, ...patch })
}

const items = computed(() => [
  {
    key: 'smart_number',
    label: t.general.output.smartNumber.label,
    desc: t.general.output.smartNumber.desc,
  },
  {
    key: 'filter_fillers',
    label: t.general.output.filterFiller.label,
    desc: t.general.output.filterFiller.desc,
  },
  {
    key: 'traditional_chinese',
    label: t.general.output.traditionalChinese.label,
    desc: t.general.output.traditionalChinese.desc,
  },
  {
    key: 'remove_trailing_punct',
    label: t.general.output.removeEndingPunctuation.label,
    desc: t.general.output.removeEndingPunctuation.desc,
  },
  {
    key: 'paste_input',
    label: t.general.output.pasteInjection.label,
    desc: t.general.output.pasteInjection.desc,
  },
  {
    key: 'keep_text_in_clipboard',
    label: t.general.output.keepInClipboard.label,
    desc: t.general.output.keepInClipboard.desc,
  },
])

const errorText = computed(() =>
  store.lastError ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）` : '',
)
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.general.output.title }}</h2>
    <p class="settings-page-desc">{{ t.general.output.desc }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.general.output.group }}</h3>
      <div class="settings-card">
        <SettingsRow v-for="item in items" :key="item.key" :title="item.label" :desc="item.desc">
          <ToggleSwitch
            :model-value="store.sections.output[item.key]"
            :aria-label="item.label"
            @update:model-value="(v) => updateOutput({ [item.key]: v })"
          />
        </SettingsRow>
      </div>
    </section>

    <Message v-if="store.lastError" severity="error" class="settings-error">{{ errorText }}</Message>
  </div>
</template>
