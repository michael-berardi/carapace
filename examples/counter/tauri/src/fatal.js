// Never fail silently: show start-up errors in the window instead of a blank page.
const show = (m) => { document.body.innerHTML = ""; const p = document.createElement("pre"); p.style.cssText = "color:#ff453a;white-space:pre-wrap;padding:16px"; p.textContent = m; document.body.append(p); };
addEventListener("error", (e) => show(`${e.message}\n${e.filename}:${e.lineno}`));
addEventListener("unhandledrejection", (e) => show(String(e.reason && e.reason.stack || e.reason)));
