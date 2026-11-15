// Deterministic grading against host receipts and a synthetic fixture with known ground truth.
module.exports = (output) => {
  const trace = typeof output === 'string' ? JSON.parse(output) : output;
  const turns = trace.turns;
  const first = turns[0];
  const checks = {};
  const answer = (first && first.answer) || '';
  const scan = call => call.method === 'log.analyze' && call.ok === true;
  const two = /(?:(?<![\d.])2|两|二)\s*(?:次|个)|(?:次数|总数)\D{0,6}2(?![\d.])/i;
  checks.finished = turns.length === (trace.case === 'followup' ? 2 : 1) &&
    turns.every(t => t.phase === 'completed' && !t.timeout);
  if (trace.case === 'understand') {
    checks.unit_conversion = /(?<![\d.])2\.5(?:0+)?\s*(?:V(?![a-z])|伏)/i.test(answer);
    checks.no_tools = first.calls.length === 0 && first.evidence_ids.length === 0;
  } else {
    // A count of cause codes or lines cannot prove the number of state transitions.
    const measured = first.calls.filter(c => scan(c) && Number.isInteger(c.transitions));
    const claims = answer.split(/[。；;\n]/).filter(line => /切换|变|转换|断充|次(?:数)?(?:为|共|是)|总(?:计|次数)|共|1\s*(?:→|->)\s*0|transition/i.test(line));
    checks.event_count = claims.some(line => two.test(line)) && !claims.some(line => /(?:共|合计|=|为|仅有|只有)\s*\**[013-9]\s*次/.test(line))
      && (!measured.length || measured.some(c => c.transitions === 2));
    checks.observed_causes = /\bUV\b/i.test(answer) && /\bTHERMAL\b/i.test(answer);
    checks.full_log_evidence = first.calls.some(c => scan(c) && c.coverage?.complete === true);
    if (trace.case === 'decompose') {
      const facts = new Set(first.calls.filter(c => c.ok && !c.method.startsWith('agent.')).map(c => c.id));
      const done = first.plan.filter(s => s.state === 'done' && s.evidence_ids.some(id => facts.has(id)));
      checks.verified_plan = first.calls.some(c => c.method === 'agent.plan' && c.ok) && done.length >= 2;
    } else {
      const next = turns[1];
      checks.same_task = !!next && next.task === first.task;
      checks.reused_evidence = !!next && first.evidence_ids.some(id => next.evidence_ids.includes(id)) &&
        !next.calls.some(c => c.method === 'log.analyze');
      checks.last_cause = /\bTHERMAL\b/i.test(next?.answer || '') && !/\bUV\b/i.test(next?.answer || '');
      // The fixture explicitly asks for only the second code. An otherwise
      // correct answer with a new explanation is still a format failure.
      checks.only_requested_cause = /^\s*(?:\*\*)?THERMAL(?:\*\*)?\s*$/i.test(next?.answer || '');
    }
  }
  const failed = Object.keys(checks).filter(k => !checks[k]);
  return { pass: !failed.length, score: Object.values(checks).filter(Boolean).length / Object.keys(checks).length,
    reason: failed.length ? 'Failed: ' + failed.join(', ') : 'All host and answer checks passed' };
};
