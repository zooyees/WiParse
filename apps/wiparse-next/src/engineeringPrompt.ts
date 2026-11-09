export function engineeringPrompt(question:string,context:string,turns:readonly {role:'user'|'assistant';text:string}[]):string{
  if(!question.trim())throw Error('请输入分析问题。');
  const history=turns.slice(-8).map(turn=>`${turn.role}:\n${turn.text}`).join('\n\n');
  const text=[history,context.trim()?`User-provided context:\n${context.trim()}`:'',`User question:\n${question.trim()}`].filter(Boolean).join('\n\n');
  if(text.includes('\0'))throw Error('发送内容包含无效字符。');
  if(new TextEncoder().encode(text).length>65536)throw Error('发送内容超过 64 KiB，请缩短问题、上下文或历史。');
  return text;
}
