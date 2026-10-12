<script setup>
/**
 * 通用 › 文本输出（T-013，F-SET-02、PRD 3.8.3）。
 * 识别标点恢复开关（T-545）：绑定 asr.punct.enabled（默认关），
 * 模型未下载时提示引导（标点恢复不生效，识别原文直出）。
 */
import { computed, onMounted, ref } from 'vue'
import ToggleSwitch from 'primevue/toggleswitch'
import Message from 'primevue/message'
import SettingsRow from '../../components/SettingsRow.vue'
import { useConfigStore } from '../../stores/config'
import { punctModelStatus } from '../../api/asr'
import { getMessage, t } from '../../i18n'

const store = useConfigStore()
/** 标点模型是否未就绪（未下载 / 损坏；状态查询失败视为就绪，不打扰）。 */
const punctModelMissing = ref(false)

onMounted(async () => {
  void store.ensureLoaded()
  try {
    const info = await punctModelStatus()
    punctModelMissing.value = !info.downloaded
  } catch {
    punctModelMissing.value = false
  }
})

function updateOutput(patch) {
  void store.updateSection('output', { ...store.sections.output, ...patch })
}

/** 识别标点恢复开关（配置在 asr 节，后端已预注册默认关）。 */
const punctEnabled = computed(() => store.sections.asr?.punct?.enabled ?? false)

function updatePunct(enabled) {
  void store.updateSection('asr', { ...store.sections.asr, punct: { enabled } })
  if (!enabled) return
  // 开启时若模型未就绪，重查一次状态刷新提示。
  void punctModelStatus()
    .then((info) => {
      punctModelMissing.value = !info.downloaded
    })
    .catch(() => undefined)
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
        <!-- 识别标点恢复（T-545）：开关在 asr 配置节，模型管理页下载标点模型后生效。 -->
        <SettingsRow
          :title="getMessage('general.punct.label')"
          :desc="getMessage('general.punct.desc')"
        >
          <ToggleSwitch
            :model-value="punctEnabled"
            :aria-label="getMessage('general.punct.label')"
            @update:model-value="(v) => updatePunct(v)"
          />
        </SettingsRow>
      </div>
      <Message
        v-if="punctEnabled && punctModelMissing"
        severity="info"
        class="settings-error"
        :closable="false"
      >
        {{ getMessage('general.punct.modelMissingHint') }}
      </Message>
    </section>

    <Message v-if="store.lastError" severity="error" class="settings-error">{{ errorText }}</Message>
  </div>
</template>
