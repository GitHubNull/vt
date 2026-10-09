/**
 * 语音输入小窗入口（tech/03 3.1）：保持极简依赖，不引 PrimeVue / router。
 */
import { createApp } from 'vue'
import MiniWindow from './MiniWindow.vue'
import '../styles/global.css'

createApp(MiniWindow).mount('#app')
