<template>
  <div class="text">{{ lyric }}</div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

const lyric = ref('')

onMounted(async () => {
  await invoke('embed_lyric_window')
  await listen('lyric-update', (event) => {
    lyric.value = event.payload
  })
})
</script>

<style scoped>
.text {
  font-family: "Microsoft YaHei", sans-serif;
  font-size: 14px;
  font-weight: bold;
  color: white;
  text-shadow: 0 1px 3px rgba(0, 0, 0, 0.8);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
  display: flex;
  justify-content: center;
  align-items: center;
  height: 100%;
}
</style>
