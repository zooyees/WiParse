const models = (process.env.WIPARSE_EVAL_MODELS || 'deepseek,qwen').split(',');
const cases = (process.env.WIPARSE_EVAL_CASES || 'understand,decompose,followup').split(',');
module.exports = {
  description: 'WiParse serial assistant, recorded live trajectories',
  prompts: ['{{scenario}}'],
  providers: models.map(model => ({
    id: 'file://./provider.cjs', label: model, config: { model },
  })),
  tests: cases.map(scenario => ({ description: scenario, vars: {scenario} })),
  defaultTest: { assert: [{ type: 'javascript', value: 'file://./score.cjs' }] },
};
