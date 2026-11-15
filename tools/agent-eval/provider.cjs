// Promptfoo replays receipts produced by WiParse's actual model and host loop.
const fs = require('node:fs');
const path = require('node:path');

module.exports = class WiParseTraceProvider {
  constructor(options) { this.model = options.config.model; }
  id() { return 'wiparse-' + this.model; }
  async callApi(prompt) {
    const scenario = prompt.trim();
    if (!['understand', 'decompose', 'followup'].includes(scenario) ||
        !['deepseek', 'qwen'].includes(this.model)) return { error: 'Unknown evaluation case' };
    const file = path.join(process.env.WIPARSE_EVAL_TRACES, this.model + '-' + scenario + '.json');
    const trace = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (trace.availability !== 'ran') return { error: trace.error || trace.availability };
    const last = trace.turns.at(-1);
    const usage = last?.usage_cumulative;
    const tokenUsage = usage?.reported_responses ? {
      prompt: usage.input_tokens, completion: usage.output_tokens,
      cached: usage.cached_input_tokens, total: usage.total_tokens,
      numRequests: usage.responses,
    } : undefined;
    return { output: JSON.stringify(trace), tokenUsage };
  }
};
