import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const el = document.getElementById('lyric');

listen('lyric-update', (event) => {
  el.textContent = event.payload;
});

invoke('embed_lyric_window');
