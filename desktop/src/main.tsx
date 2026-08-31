import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { createMockBridge, createTauriBridge } from './bridge';
import { App } from './app/App';
import { ErrorBoundary } from './app/ErrorBoundary';
import './styles/tokens.css';
import './styles/reset.css';
import './styles/base.css';
import './styles/layout.css';
import './styles/themes.css';

// Tauri's bundled API uses the internal bridge, while the Windows WebView2
// page can expose the Tauri origin before that property is observable to the
// entry module. Keep browser/Playwright runs on the mock bridge, but recognize
// both stable Tauri origins for the packaged shell.
const isTauri =
  '__TAURI_INTERNALS__' in window ||
  window.location.protocol === 'tauri:' ||
  window.location.hostname === 'tauri.localhost';
const bridge = isTauri ? createTauriBridge() : createMockBridge();
const root = document.getElementById('root');

if (!root) throw new Error('desktop root element is missing');

createRoot(root).render(
  <StrictMode>
    <ErrorBoundary>
      <App bridge={bridge} />
    </ErrorBoundary>
  </StrictMode>,
);
