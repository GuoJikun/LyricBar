<template>
  <div class="lyric-container" v-show="lyric">
    <div class="lyric-text">{{ lyric }}</div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { listen } from '@tauri-apps/api/event'

const lyric = ref('')

onMounted(async () => {
  await listen('lyric-update', (event) => {
    lyric.value = event.payload
  })
})
</script>

<style>
.lyric-container {
  width: 100%;
  height: 100%;
  display: flex;
  justify-content: center;
  align-items: center;
  background: transparent;
}

.lyric-text {
  font-family: "Microsoft YaHei", sans-serif;
  font-size: 14px;
  font-weight: bold;
  color: white;
  text-shadow: 0 1px 3px rgba(0, 0, 0, 0.8);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
  padding: 0 8px;
}
</style>
