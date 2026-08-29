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

const isTauri = '__TAURI_INTERNALS__' in window;
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
