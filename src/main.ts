import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';

// Keep the window behaving like an app rather than a web page:
// no reload, print, find bar, back/forward or right-click menu.
const blockedWithCtrl = new Set(['r', 'p', 'f', 'g', 'u', 'j', 's', 'o', 'n', 'h', 'l']);
window.addEventListener('keydown', (e) => {
  const key = e.key.toLowerCase();
  if (e.key === 'F5' || e.key === 'F3' || e.key === 'F7') e.preventDefault();
  if ((e.ctrlKey || e.metaKey) && blockedWithCtrl.has(key)) e.preventDefault();
  if (e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) e.preventDefault();
});

window.addEventListener('contextmenu', (e) => {
  const target = e.target as HTMLElement | null;
  const editable = target?.closest('input, textarea, .selectable');
  if (!editable) e.preventDefault();
});

for (const type of ['mousedown', 'mouseup', 'auxclick'] as const) {
  window.addEventListener(type, (e) => {
    if (e.button === 3 || e.button === 4) e.preventDefault();
  });
}

const target = document.getElementById('app');
if (target) mount(App, { target });
