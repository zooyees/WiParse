export type EvidenceTarget = 'anchor'|'document'|'image'|'wave'|'external'|'blocked';

/** Decide presentation only; the native side revalidates paths and session. */
export function evidenceTarget(source:string):EvidenceTarget {
  if (!source || source.length>4096 || /[\u0000-\u001f]/.test(source)) return 'blocked';
  if (source.startsWith('#')) return 'anchor';
  let decoded:string;
  try { decoded=decodeURIComponent(source.split(/[?#]/)[0]); } catch { return 'blocked'; }
  if (/^https?:\/\//i.test(source)) return 'external';
  if (/^(?:\/\/|\\\\)/.test(decoded)) return 'blocked';
  if (/^[a-z][a-z\d+.-]*:/i.test(decoded) && !/^[a-z]:[\\/]/i.test(decoded)) return 'blocked';
  const extension=decoded.split('.').pop()?.toLowerCase();
  if (['png','jpg','jpeg','gif','webp'].includes(extension??'')) return 'image';
  if (['csv','isf','wfm'].includes(extension??'')) return 'wave';
  if (['md','markdown','txt','log','json','jsonl'].includes(extension??'')) return 'document';
  if (['pdf','html','htm'].includes(extension??'')) return 'external';
  return 'blocked';
}

export function headingAnchor(text:string):string {
  return text.trim().toLocaleLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu,'').replace(/\s+/g,'-');
}

type MarkdownNode={type:string;value?:string;alt?:string;children?:MarkdownNode[];data?:{hProperties?:Record<string,unknown>}};

/** Assign unique anchors to every heading level, including formatted titles. */
export function reportHeadingIds() {
  return (tree:MarkdownNode)=>{
    const used=new Set<string>(),pending=[tree];
    while(pending.length){
      const node=pending.pop()!;
      if(node.type==='heading'){
        const text:string[]=[];const parts=[...(node.children??[])].reverse();
        while(parts.length){const part=parts.pop()!;if(part.value)text.push(part.value);else if(part.alt)text.push(part.alt);if(part.children)parts.push(...[...part.children].reverse());}
        const base=headingAnchor(text.join(''))||'section';let id=base,index=0;
        while(used.has(id))id=`${base}-${++index}`;
        used.add(id);node.data={...node.data,hProperties:{...node.data?.hProperties,id}};
      }
      if(node.children)pending.push(...[...node.children].reverse());
    }
  };
}
