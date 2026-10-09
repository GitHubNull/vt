/**
 * 设置主窗口入口（tech/03 3.2）。
 */
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import PrimeVue from 'primevue/config'
import App from './App.vue'
import { createAppRouter } from './router'
import 'primeicons/primeicons.css'
import './styles/primevue-theme.css'
import './styles/global.css'

const app = createApp(App)

app.use(createPinia())
app.use(createAppRouter())
app.use(PrimeVue)

app.mount('#app')
