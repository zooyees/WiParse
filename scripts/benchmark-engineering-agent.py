"""Run the engineering agent's deterministic seed checks from this checkout."""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEEDS = [
    ("serial", "offline-state-count", "core", "log_analysis::events::tests::generic_state_rules_count_events_not_alarm_lines_and_keep_entities_separate"),
    ("serial", "file-boundaries", "core", "log_analysis::events::tests::state_never_crosses_file_boundary"),
    ("serial", "full-file-evidence", "core", "log_analysis::tests::full_file_analysis_finds_evidence_outside_recent_display_and_bounds_output"),
    ("data", "source-row-and-chart", "gui", "data_analysis::ai_extract::tests::extraction_status_and_source_bound_dataset"),
    ("data", "cycle-power-units", "core", "log_analysis::timeline::tests::cycle_uses_actual_timestamps_units_and_only_same_line_power"),
    ("data", "cross-clock", "core", "ai::workflow::engineering_fit_tests::incomplete_deliverables_and_alignment_require_evidence"),
    ("wave", "synchronous-capture", "core", "engineering_wave::quality_regressions::synchronous_deadtime_is_quantized_and_wrong_timebase_is_rejected"),
    ("wave", "deadtime-overlap", "core", "automation::analysis::tests::deadtime_bidirectional_overlap_and_provenance"),
    ("report", "pagination", "gui", "test_report::ai_read::tests::report_pages_preserve_original_lines_and_cancel"),
    ("agent", "partial-negation", "core", "ai::workflow::engineering_fit_tests::scoped_amendments_preserve_unrelated_deliverables"),
    ("agent", "provider-recovery", "gui", "serial_tool::ai_chat::protocol::recovery_tests::retry_matches_transport_and_preserves_json_execution"),
    ("agent", "scope-revocation", "gui", "serial_tool::ai_chat::view_tests::source_change_clears_old_context_and_control_authorization"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    results = []
    for category, key, package, test in SEEDS:
        target = ["--lib"] if package == "core" else ["--bin", "wiparse-gui"]
        run = subprocess.run(
            ["cargo", "test", "--offline", "--locked", "-p", "wiparse-" + package,
             *target, test, "--", "--exact"], cwd=ROOT, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, encoding="utf-8", errors="replace")
        passed = run.returncode == 0 and re.search(r"^test " + re.escape(test) +
                                                  r" \.\.\. ok\s*$", run.stdout, re.M) is not None
        results.append(dict(category=category, id=key, test=test, passed=passed))
        if not passed:
            print(run.stdout[-2500:], file=sys.stderr)
    args.output.write_text(json.dumps(dict(kind="deterministic_software_regression",
                                           cases=results), indent=2), encoding="utf-8")
    return int(any(not row["passed"] for row in results))


if __name__ == "__main__":
    sys.exit(main())
