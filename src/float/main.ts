/**
 * 录音悬浮球窗口入口（tech/03 3.1）：保持极简依赖，不引 PrimeVue / router。
 */
import { createApp } from 'vue'
import FloatBall from './FloatBall.vue'
import '../styles/global.css'

createApp(FloatBall).mount('#app')
