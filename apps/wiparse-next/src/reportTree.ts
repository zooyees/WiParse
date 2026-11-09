export type TreeFile = {name: string; path: string; mtime: number | null; view?: string; group?: string};

const markdown = (name: string) => /\.(md|markdown)$/i.test(name);

/** Same choice as `IndexedResult::latest_report`: markdown only, path order, last equal mtime wins. */
export function latestMarkdown(files: TreeFile[]): TreeFile | null {
  const sorted = files.filter(file => markdown(file.name)).slice().sort((left, right) => left.path < right.path ? -1 : left.path > right.path ? 1 : 0);
  const unique: TreeFile[] = [];
  for (const file of sorted) {
    if (unique.at(-1)?.path === file.path) continue;
    unique.push(file);
  }
  return unique.reduce<TreeFile | null>((best, file) => !best || (file.mtime ?? -1) >= (best.mtime ?? -1) ? file : best, null);
}

/** Project and test ids accepted by the confirmed remove command. */
export function canRemoveProjectTest(project: string, id: string): boolean {
  const valid = (value: string) => value.length <= 128 && /^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(value);
  return valid(project) && valid(id);
}

/** Same choice as `IndexedTest::latest_report`: walk runs in snapshot order. */
function fileStem(name: string): string {
  const base = name.split(/[/\\]/).pop() ?? name;
  const dot = base.lastIndexOf('.');
  return dot > 0 ? base.slice(0, dot) : base;
}

function fileExtension(name: string): string {
  const base = name.split(/[/\\]/).pop() ?? name;
  const dot = base.lastIndexOf('.');
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : '';
}

/** Same shortening as the old report tree before the summary title is applied. */
export function artifactTitle(name: string, prefix: string, stamp: string): string {
  let body = fileStem(name);
  if (prefix && body.startsWith(prefix)) body = body.slice(prefix.length).replace(/^_+/, '');
  if (stamp.length === 15 && stamp[8] === '_') {
    const token = `_${stamp.slice(0, 8)}_`;
    const index = body.indexOf(token);
    if (index >= 0) {
      const head = body.slice(0, index).replace(/_+$/, '');
      const after = body.slice(index + token.length);
      const hhmmss = after.length >= 6 && /^[0-9]{6}/.test(after) ? after.slice(0, 6) : stamp.slice(9);
      const nice = `${hhmmss.slice(0, 2)}:${hhmmss.slice(2, 4)}:${hhmmss.slice(4, 6)}`;
      body = head ? `${head} ${nice}` : nice;
      return body.startsWith('summary') ? `总报告${body.slice(7).replace(/\.md$/, '')}` : body;
    }
  }
  const extension = fileExtension(name);
  const label = !body ? name : extension ? `${body}.${extension}` : body;
  return label.startsWith('summary') ? `总报告${label.slice(7).replace(/\.md$/, '')}` : label;
}

export function latestRunReport(runs: {latest: TreeFile | null}[]): TreeFile | null {
  return runs.reduce<TreeFile | null>((best, run) => !run.latest ? best : !best || (run.latest.mtime ?? -1) >= (best.mtime ?? -1) ? run.latest : best, null);
}

/** A confirmation belongs to the exact project that produced its indexed row. */
export function reportProjectMatches(current: string, indexed: string, confirmed = current): boolean {
  return canRemoveProjectTest(current, 'test') && current === indexed && current === confirmed;
}
