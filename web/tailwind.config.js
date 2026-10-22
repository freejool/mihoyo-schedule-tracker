/** @type {import('tailwindcss').Config} */
export default {
  theme: {
    extend: {
      colors: {
        surface: {
          DEFAULT: 'var(--color-surface)',
          raised: 'var(--color-surface-raised)',
          muted: 'var(--color-surface-muted)'
        },
        ink: {
          DEFAULT: 'var(--color-ink)',
          muted: 'var(--color-ink-muted)',
          faint: 'var(--color-ink-faint)'
        },
        border: {
          DEFAULT: 'var(--color-border)'
        },
        accent: {
          DEFAULT: 'var(--color-accent)'
        },
        status: {
          official: 'var(--color-status-official)',
          manual: 'var(--color-status-manual)',
          predicted: 'var(--color-status-predicted)'
        }
      }
    }
  }
};
