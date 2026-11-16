/**
 * The site's whole interactive layer outside the plugin directory:
 * theme toggle, mobile menu, remembered language, copy buttons.
 * Everything degrades to working links and readable code without it.
 */

const root = document.documentElement;

function store(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage can be unavailable (private mode, blocked): the choice then
    // lasts for this page only.
  }
}

function currentTheme(): 'light' | 'dark' {
  const chosen = root.dataset.theme;
  if (chosen === 'light' || chosen === 'dark') return chosen;
  return matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function syncThemeToggles(): void {
  const theme = currentTheme();
  for (const button of document.querySelectorAll<HTMLButtonElement>('[data-theme-toggle]')) {
    const label = theme === 'dark' ? button.dataset.labelLight : button.dataset.labelDark;
    if (label) button.setAttribute('aria-label', label);
    button.setAttribute('aria-pressed', String(theme === 'dark'));
  }
}

for (const button of document.querySelectorAll<HTMLButtonElement>('[data-theme-toggle]')) {
  button.addEventListener('click', () => {
    const next = currentTheme() === 'dark' ? 'light' : 'dark';
    root.dataset.theme = next;
    store('dp-theme', next);
    syncThemeToggles();
  });
}
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', syncThemeToggles);
syncThemeToggles();

const navToggle = document.querySelector<HTMLButtonElement>('[data-nav-toggle]');
const mobileNav = document.getElementById('mobile-nav');
if (navToggle && mobileNav) {
  const setOpen = (open: boolean) => {
    mobileNav.hidden = !open;
    navToggle.setAttribute('aria-expanded', String(open));
    const label = open ? navToggle.dataset.labelClose : navToggle.dataset.labelOpen;
    if (label) navToggle.setAttribute('aria-label', label);
    navToggle.querySelector<SVGElement>('.icon-open')?.style.setProperty('display', open ? 'none' : '');
    navToggle.querySelector<SVGElement>('.icon-close')?.style.setProperty('display', open ? '' : 'none');
  };
  navToggle.addEventListener('click', () => setOpen(mobileNav.hidden !== false));
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !mobileNav.hidden) {
      setOpen(false);
      navToggle.focus();
    }
  });
  matchMedia('(min-width: 60rem)').addEventListener('change', (event) => {
    if (event.matches) setOpen(false);
  });
}

for (const link of document.querySelectorAll<HTMLAnchorElement>('[data-set-lang]')) {
  link.addEventListener('click', () => store('dp-lang', link.dataset.setLang ?? 'en'));
}

// Code fences in Markdown: add the copy button without moving anything.
const labels = document.body.dataset;
for (const pre of document.querySelectorAll<HTMLPreElement>('.prose pre')) {
  const wrap = document.createElement('div');
  wrap.className = 'code-block code-block-bare';
  pre.replaceWith(wrap);
  wrap.append(pre);
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'code-copy';
  button.dataset.copy = '';
  button.dataset.copied = labels.labelCopied ?? '';
  button.dataset.failed = labels.labelCopyFailed ?? '';
  const label = document.createElement('span');
  label.dataset.copyLabel = '';
  label.setAttribute('aria-live', 'polite');
  label.textContent = labels.labelCopy ?? '';
  button.append(label);
  wrap.append(button);
}

// Wide tables scroll inside themselves; make that region reachable by keyboard.
let tableNumber = 0;
for (const table of document.querySelectorAll<HTMLTableElement>('.prose table')) {
  tableNumber++;
  if (table.scrollWidth > table.clientWidth) {
    table.tabIndex = 0;
    table.setAttribute('role', 'region');
    table.setAttribute('aria-label', `${labels.labelTable ?? ''} ${tableNumber}`);
  }
}

for (const button of document.querySelectorAll<HTMLButtonElement>('[data-copy]')) {
  const label = button.querySelector<HTMLElement>('[data-copy-label]');
  const idle = label?.textContent ?? '';
  let timer: number | undefined;
  button.addEventListener('click', async () => {
    const code = button.closest('.code-block')?.querySelector('pre')?.textContent ?? '';
    let message = button.dataset.copied ?? '';
    try {
      await navigator.clipboard.writeText(code);
    } catch {
      message = button.dataset.failed ?? '';
    }
    if (label) label.textContent = message;
    window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      if (label) label.textContent = idle;
    }, 2000);
  });
}
