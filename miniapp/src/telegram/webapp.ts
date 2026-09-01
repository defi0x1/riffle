/**
 * Thin wrapper around window.Telegram.WebApp, loaded by the <script> tag in index.html. This
 * module never validates initData itself -- it only reads the raw string Telegram attaches to
 * the launch and hands it to the backend on every request that needs it. The backend recomputes
 * the HMAC and checks freshness; this app treats initData as opaque, unverified launch context,
 * never as a local source of truth for who is using it. See README.md for the verification
 * contract this assumes on the backend side.
 */

interface TelegramWebApp {
  initData: string;
  initDataUnsafe: unknown;
  ready(): void;
  expand(): void;
  colorScheme: "light" | "dark";
  themeParams: Record<string, string>;
  MainButton: {
    show(): void;
    hide(): void;
    setText(text: string): void;
    onClick(cb: () => void): void;
    offClick(cb: () => void): void;
  };
  HapticFeedback?: {
    notificationOccurred(type: "error" | "success" | "warning"): void;
  };
}

declare global {
  interface Window {
    Telegram?: { WebApp?: TelegramWebApp };
  }
}

export function getTelegramWebApp(): TelegramWebApp | null {
  return window.Telegram?.WebApp ?? null;
}

/**
 * Raw, unverified launch payload. Never trust anything derived from this client-side for an
 * access-control decision -- send it to the backend and let the backend's HMAC check (keyed on
 * the bot token, which this app never has) decide.
 */
export function getInitData(): string {
  return getTelegramWebApp()?.initData ?? "";
}

export function initTelegramApp(): void {
  const app = getTelegramWebApp();
  if (!app) return; // running outside Telegram (local dev) -- nothing to initialise
  app.ready();
  app.expand();
}
