/** @type {import('tailwindcss').Config} */
module.exports = {
  darkMode: 'class',
  content: [
    "./crates/savevault-app/ui/frontend/index.html",
    "./crates/savevault-app/ui/frontend/app.js"
  ],
  theme: {
    extend: {
      colors: {
        app: {
          bg: '#f8fafc',
          card: '#ffffff',
          sidebar: '#f1f5f9',
          border: 'rgba(203,213,225,0.7)',
          emeraldBadge: '#dcfce7',
          emeraldText: '#15803d',
        }
      },
      fontFamily: { sans: ['Inter', 'sans-serif'] },
      boxShadow: { xs: '0 1px 3px rgba(0,0,0,0.06)' }
    }
  },
  plugins: [],
};
