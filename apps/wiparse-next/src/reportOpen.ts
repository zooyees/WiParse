export type ArtifactTarget = 'report' | 'text' | 'wave' | 'external';

function extension(name: string): string {
  const base = name.split(/[/\\]/).pop() ?? name;
  const dot = base.lastIndexOf('.');
  return dot <= 0 ? '' : base.slice(dot + 1).toLowerCase();
}

function named(view: string): ArtifactTarget {
  if (view === 'wave') return 'wave';
  if (view === 'report') return 'report';
  if (view === 'log') return 'text';
  return 'external';
}

/** Same choice as the old report page `open_view_for_file`. */
export function openTarget(name: string, fileView: string, groupView: string): ArtifactTarget {
  switch (extension(name)) {
    case 'md':
    case 'markdown':
      return 'report';
    case 'png':
    case 'jpg':
    case 'jpeg':
    case 'gif':
    case 'webp':
    case 'bmp':
    case 'pdf':
    case 'html':
    case 'htm':
    case 'svg':
      return 'external';
    case 'isf':
    case 'wfm':
    case 'csv':
      return 'wave';
    case 'log':
    case 'json':
    case 'jsonl':
      return 'text';
    case 'txt':
      return named(groupView);
    default:
      return named(fileView);
  }
}

export function targetTitle(target: ArtifactTarget): string {
  if (target === 'wave') return '在波形分析中打开';
  if (target === 'external') return '用系统程序打开';
  if (target === 'report') return '预览报告';
  return '预览文本';
}
