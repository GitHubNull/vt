<script setup lang="ts">
import { ref } from 'vue'
import Button from 'primevue/button'
import { ping } from '../api/client'
import { t } from '../i18n'

const pingResult = ref('')
const pingError = ref('')
const testing = ref(false)

async function onPing() {
  testing.value = true
  pingResult.value = ''
  pingError.value = ''
  try {
    pingResult.value = await ping()
  } catch (error) {
    pingError.value = error instanceof Error ? error.message : String(error)
  } finally {
    testing.value = false
  }
}
</script>

<template>
  <main class="home-view">
    <p class="hint">{{ t.app.scaffoldHint }}</p>
    <Button :label="t.settings.pingButton" :loading="testing" @click="onPing" />
    <p v-if="pingResult" class="ping-result" data-testid="ping-result">{{ pingResult }}</p>
    <p v-if="pingError" class="ping-error">{{ pingError }}</p>
  </main>
</template>
