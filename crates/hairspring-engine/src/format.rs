//! Human, JUnit and SARIF renderings of run outcomes.
use crate::report::{RunOutcome, RunStatus};
use serde_json::json;

pub fn to_text(target: &str, outcome: &RunOutcome) -> String {
    let mut out = String::new();
    match (&outcome.status, &outcome.failure) {
        (RunStatus::Failed, Some(f)) => {
            out.push_str(&format!(
                "FAILED  {target} ({})\n  {}\n  steps ({}):\n",
                f.backend,
                f.message,
                f.steps.len()
            ));
            for s in &f.steps {
                out.push_str(&format!("    {:>3}. {}\n", s.index + 1, s.label));
            }
            out.push_str("  bundle:\n");
            for line in f.bundle.lines() {
                out.push_str(&format!("    {line}\n"));
            }
        }
        (RunStatus::Error, _) => out.push_str(&format!(
            "ERROR   {target}: {}\n",
            outcome.error.as_deref().unwrap_or("unknown error")
        )),
        _ => out.push_str(&format!(
            "PASSED  {target}  ({} steps, {} trials, {} ms)\n",
            outcome.steps_executed, outcome.trials, outcome.elapsed_ms
        )),
    }
    out.push_str(&crate::format_props::sections(outcome));
    out
}

fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            // XML 1.0 forbids most control characters; drop them rather than emit invalid XML.
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t') => String::new(),
            c => c.to_string(),
        })
        .collect()
}

pub fn to_junit(results: &[(String, RunOutcome)]) -> String {
    let failures = results
        .iter()
        .filter(|(_, o)| o.status == RunStatus::Failed)
        .count();
    let errors = results
        .iter()
        .filter(|(_, o)| o.status == RunStatus::Error)
        .count();
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuite name=\"hairspring\" tests=\"{}\" failures=\"{failures}\" errors=\"{errors}\">\n",
        results.len()
    ));
    for (target, o) in results {
        let name = xml_escape(target);
        out.push_str(&format!(
            "  <testcase classname=\"hairspring\" name=\"{name}\" time=\"{:.3}\">\n",
            o.elapsed_ms as f64 / 1000.0
        ));
        match (&o.status, &o.failure) {
            (RunStatus::Failed, Some(f)) => {
                let body: String = f
                    .steps
                    .iter()
                    .map(|s| format!("{}. {}\n", s.index + 1, s.label))
                    .collect::<String>()
                    + &f.bundle;
                out.push_str(&format!(
                    "    <failure message=\"{}\">{}</failure>\n",
                    xml_escape(&f.message),
                    xml_escape(&body)
                ));
            }
            (RunStatus::Error, _) => {
                out.push_str(&format!(
                    "    <error message=\"{}\"/>\n",
                    xml_escape(o.error.as_deref().unwrap_or("error"))
                ));
            }
            _ => {}
        }
        out.push_str("  </testcase>\n");
    }
    out.push_str("</testsuite>\n");
    out
}

pub fn to_sarif(results: &[(String, RunOutcome)]) -> String {
    let items: Vec<_> = results
        .iter()
        .filter(|(_, o)| o.status != RunStatus::Passed)
        .map(|(target, o)| {
            let (rule, text) = match (&o.status, &o.failure) {
                (RunStatus::Failed, Some(f)) => ("hairspring/violation", format!("{target}: {} ({} steps)", f.message, f.shrunk_len)),
                _ => ("hairspring/error", format!("{target}: {}", o.error.as_deref().unwrap_or("error"))),
            };
            json!({
                "ruleId": rule,
                "level": "error",
                "message": { "text": text },
                "locations": [{ "physicalLocation": { "artifactLocation": { "uri": "hairspring.toml" } } }]
            })
        })
        .collect();
    let doc = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": { "name": "hairspring", "rules": [
                { "id": "hairspring/violation", "shortDescription": { "text": "A property was violated" } },
                { "id": "hairspring/error", "shortDescription": { "text": "The run could not complete" } }
            ] } },
            "results": items
        }]
    });
    serde_json::to_string_pretty(&doc).expect("SARIF document serialises")
}
