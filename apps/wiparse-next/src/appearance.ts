export function applyAppearance(theme:string) {
  document.documentElement.dataset.theme=theme==='dark'?'dark':'light';
}
