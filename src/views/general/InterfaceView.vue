<script setup>
/**
 * 通用 › 界面（T-013，F-SET-01）：界面语言 / 主题。
 * v0.1 仅简体中文与浅色可用，其余选项置灰标注 v1.0（T-013 SPEC 1.2）。
 * 约定：ESLint 无 TS parser（tech/2.2），SFC script 保持纯 JS 语法，类型集中在 api/types.ts。
 */
import { computed, onMounted } from 'vue'
import Select from 'primevue/select'
import Message from 'primevue/message'
import SettingsRow from '../../components/SettingsRow.vue'
import { useConfigStore } from '../../stores/config'
import { t } from '../../i18n'

const store = useConfigStore()

onMounted(() => {
  void store.ensureLoaded()
})

const languageOptions = computed(() => [
  { label: t.general.interface.language.options.system, value: 'system', disabled: false },
  { label: t.general.interface.language.options.zhCN, value: 'zh_cn', disabled: false },
  { label: t.general.interface.language.options.enUS, value: 'en_us', disabled: true },
  { label: t.general.interface.language.options.zhTW, value: 'zh_tw', disabled: true },
])

const themeOptions = computed(() => [
  { label: t.general.interface.theme.options.system, value: 'system', disabled: false },
  { label: t.general.interface.theme.options.light, value: 'light', disabled: false },
  { label: t.general.interface.theme.options.dark, value: 'dark', disabled: true },
])

function updateLanguage(value) {
  void store.updateSection('general', { ...store.sections.general, language: value })
}

function updateTheme(value) {
  void store.updateSection('general', { ...store.sections.general, theme: value })
}

const errorText = computed(() =>
  store.lastError ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）` : '',
)
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.general.interface.title }}</h2>
    <p class="settings-page-desc">{{ t.general.interface.desc }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.general.interface.group }}</h3>
      <div class="settings-card">
        <SettingsRow
          :title="t.general.interface.language.label"
          :desc="t.general.interface.language.desc"
        >
          <Select
            :model-value="store.sections.general.language"
            :options="languageOptions"
            option-label="label"
            option-value="value"
            option-disabled="disabled"
            :aria-label="t.general.interface.language.label"
            @update:model-value="updateLanguage"
          />
        </SettingsRow>
        <SettingsRow :title="t.general.interface.theme.label" :desc="t.general.interface.theme.desc">
          <Select
            :model-value="store.sections.general.theme"
            :options="themeOptions"
            option-label="label"
            option-value="value"
            option-disabled="disabled"
            :aria-label="t.general.interface.theme.label"
            @update:model-value="updateTheme"
          />
        </SettingsRow>
      </div>
    </section>

    <Message v-if="store.lastError" severity="error" class="settings-error">{{ errorText }}</Message>
  </div>
</template>
