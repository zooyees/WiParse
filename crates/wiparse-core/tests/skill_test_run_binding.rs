use serde_json::{json, Value};
use wiparse_core::ai::workflow::Workflow;

fn result_plan(report_id: &str) -> Value {
    json!({"steps":[
        {"id":"skill_test_execution","title":"测试执行辅助","state":"done","deliverable":"operation","evidence_ids":["wait"]},
        {"id":"skill_test_result","title":"测试结果分析","state":"done","deliverable":"analysis","evidence_ids":[report_id]}
    ]})
}

#[test]
fn failed_process_can_be_investigated_but_a_different_runs_report_cannot() {
    let mut workflow=Workflow::default();
    workflow.skill_ids=vec!["test_execution".into(),"test_result".into()];
    workflow.initialize_skill_plan().unwrap();
    workflow.record_call("start","testing.run",&json!({}),&json!({"ok":true,"data":{"run_id":"run-a","state":"accepted"}}));
    workflow.record_call("wait","testing.wait",&json!({"run_id":"run-a"}),&json!({"ok":false,"data":{
        "process_completed":true,"execution_succeeded":false,
        "status":{"completed_run_identity":"run-a","process_exit_code":1}}}));
    let report=|id:&str|json!({"ok":true,"data":{"source_id":"report-a","source_version":"version-a","source_bytes":32,
        "run_provenance":{"host_run_id":id},"lines":[{"line":1,"text":"failure"}],"reached_eof":true}});
    workflow.record_call("wrong","report.read",&json!({}),&report("run-b"));
    assert!(workflow.update(result_plan("wrong")).is_err());
    workflow.record_call("correct","report.read",&json!({}),&report("run-a"));
    assert!(workflow.update(result_plan("correct")).is_ok());
}
