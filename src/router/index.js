import { createRouter, createWebHistory } from 'vue-router'
import HomeView from '../views/HomeView.vue'
import LyricView from '../views/LyricView.vue'

const routes = [
  { path: '/', component: HomeView },
  { path: '/lyric', component: LyricView },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

export default router
