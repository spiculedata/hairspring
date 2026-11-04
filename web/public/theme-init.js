/* Sets the saved colour theme before the page is painted, so a reader who chose light or dark does not see a flash of
   the other. Kept in step with src/shell/theme.ts (a test checks the key and values). It is a separate file because the
   server's Content-Security-Policy does not allow inline scripts. */
(function () {
  try {
    var mode = localStorage.getItem('hairspring-theme');
    if (mode === 'light' || mode === 'dark') document.documentElement.setAttribute('data-theme', mode);
  } catch (e) {
    /* Storage is blocked: the system setting applies until the app starts. */
  }
})();
