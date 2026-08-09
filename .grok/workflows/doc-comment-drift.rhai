let meta = #{
    name: "doc-comment-drift",
    description: "Find and fix documentation/comment drift after K4 HW + K8 QEMU slices",
    phases: [
        #{ title: "Scan", detail: "parallel drift hunters across status, ADRs, comments, cross-docs" },
        #{ title: "Verify", detail: "adversarial check of each finding against the tree" },
        #{ title: "Fix", detail: "apply confirmed fixes; re-run doc gates" },
    ],
    when_to_use: "After multi-ADR landings (0067–0070) when gates are green but prose/comments may lag",
};

// Structured findings from scanners.
let findings_schema = #{
    "type": "object",
    "required": ["findings"],
    "properties": #{
        "findings": #{
            "type": "array",
            "maxItems": 12,
            "items": #{
                "type": "object",
                "required": ["path", "issue", "suggested_fix", "severity"],
                "properties": #{
                    "path": #{ "type": "string" },
                    "issue": #{ "type": "string" },
                    "suggested_fix": #{ "type": "string" },
                    "severity": #{ "type": "string" },
                },
            },
        },
        "notes": #{ "type": "string" },
    },
};

let verdict_schema = #{
    "type": "object",
    "required": ["real", "reason", "evidence"],
    "properties": #{
        "real": #{ "type": "boolean" },
        "reason": #{ "type": "string" },
        "evidence": #{ "type": "string" },
    },
};

let fix_schema = #{
    "type": "object",
    "required": ["fixed", "skipped", "summary"],
    "properties": #{
        "fixed": #{
            "type": "array",
            "maxItems": 20,
            "items": #{ "type": "string" },
        },
        "skipped": #{
            "type": "array",
            "maxItems": 20,
            "items": #{ "type": "string" },
        },
        "summary": #{ "type": "string" },
    },
};

// Ground truth the scanners must treat as authoritative (post-landing truth).
let truth = "";
truth += "AUTHORITATIVE STATUS (as of commits through ADR-0070, do not invent older states):\n";
truth += "- K4 budget + EL0 preemption + EL1 preemption: done (HW). Transcripts 20260809-122251 (EL0) and 20260809-151021 (EL1).\n";
truth += "- K8 first slice (unpark core1 idle): done (QEMU) via ADR-0070; HW stamp residual; NO per-core runqueue yet.\n";
truth += "- ADR-0067: lab QEMU x86 intent only, no x86 code. ADR-0069: host-class north star; name remains Harbor (not HarbOS).\n";
truth += "- Product board today remains Raspberry Pi 4B (ADR-0007). Multi-arch ready structure, not multi-product.\n";
truth += "- ADR-0006 cooperative model partially superseded by K4 preemption on IRQ epilogue (EL0+EL1); device IRQ handlers still never switch.\n";
truth += "- Secondary cores: QEMU uses spin-table PA 0xe0; real Pi uses secondary_entry table. smp: core1 alive is the oracle.\n";
truth += "- Automated gates xrefs/doc-claims/doc-symbols/roadmap-evidence were clean before this run; still hunt prose/comment lag.\n";
truth += "OUT OF SCOPE: rewriting code logic, inventing new features, renaming Harbor, claiming K8 HW done.\n";

phase("Scan");

let scan_jobs = [];

let p0 = "";
p0 += "You are a documentation drift hunter for harbor-kernel (Rust bare-metal microkernel).\n";
p0 += truth;
p0 += "TASK: Search docs/ and SECURITY.md / README.md for STALE STATUS claims.\n";
p0 += "Use grep and read_file on the real tree. Look for: 'HW stamp pending' for ADR-0068, ";
p0 += "'done (QEMU)' for EL1 preemption when HW is done, 'same-EL in design', 'K8 in design' without 0070, ";
p0 += "'code deferred' for K8 if 0070 landed, 'HarbOS', multi-arch marketed as product, ";
p0 += "cooperative-only claims that ignore K4 IRQ preemption.\n";
p0 += "Return at most 12 findings as {path, issue, suggested_fix, severity: high|medium|low}. ";
p0 += "Empty findings only after searching. Prefer concrete file paths.";
scan_jobs.push(#{
    prompt: p0,
    label: "scan:status",
    capability_mode: "read-only",
    output_schema: findings_schema,
});

let p1 = "";
p1 += "You are a source-comment drift hunter for harbor-kernel.\n";
p1 += truth;
p1 += "TASK: Grep src/ (and crates/ if needed) comments/docs for stale claims: ";
p1 += "'single core only' as permanent, 'secondary cores remain parked forever', ";
p1 += "'no SMP', 'cooperative only / never preempt' ignoring ADR-0064/0068, ";
p1 += "wrong GIC/timer stories, 'kernel8.img is universal'.\n";
p1 += "Read the actual files around matches. Report at most 12 {path, issue, suggested_fix, severity}.\n";
p1 += "Empty list only after real searches. Do not flag accurate historical comments that say residual.";
scan_jobs.push(#{
    prompt: p1,
    label: "scan:comments",
    capability_mode: "read-only",
    output_schema: findings_schema,
});

let p2 = "";
p2 += "You are a cross-document consistency reviewer for harbor-kernel.\n";
p2 += truth;
p2 += "TASK: Read docs/roadmap.md, docs/vision.md, docs/architecture.md (preemption/SMP sections), ";
p2 += "docs/verification.md (ADR table rows 0068/0070), docs/stack.md, docs/adr/README.md, ";
p2 += "docs/adr/0048, 0067, 0069, 0070 headers/status.\n";
p2 += "Find contradictions between these sources (e.g. roadmap vs verification, vision H3 vs ADR-0007, ";
p2 += "architecture table missing 0070, facade smp vs arch-contract).\n";
p2 += "Max 12 findings {path, issue, suggested_fix, severity}. Use tools; no memory-only answers.";
scan_jobs.push(#{
    prompt: p2,
    label: "scan:crossdoc",
    capability_mode: "read-only",
    output_schema: findings_schema,
});

let p3 = "";
p3 += "You are an ADR/index hygiene checker for harbor-kernel.\n";
p3 += truth;
p3 += "TASK: Ensure every ADR 0067-0070 is listed in docs/adr/README.md and docs/architecture.md artefact table ";
p3 += "with matching status wording; related: links in 0048/0006/0051 mention successors where needed; ";
p3 += "docs/design/native-multiarch-practices.md and host-lab-platform-matrix.md do not claim code exists for x86.\n";
p3 += "Grep and read files. Max 12 findings {path, issue, suggested_fix, severity}.";
scan_jobs.push(#{
    prompt: p3,
    label: "scan:adr-index",
    capability_mode: "read-only",
    output_schema: findings_schema,
});

let scans = parallel(scan_jobs);

let all_findings = [];
for r in scans {
    if r != () && r.success && r.output.findings != () {
        for f in r.output.findings {
            all_findings.push(f);
        }
    }
}

log("scan collected " + all_findings.len().to_string() + " raw findings");

if all_findings.len() == 0 {
    let report = "# Doc/comment drift report\n\nScanners found no raw findings. Automated gates were already clean.\n";
    let path = write_scratch_file("drift-report.md", report);
    complete(#{
        summary: "No drift findings from scanners",
        path: path,
        confirmed: 0,
        fixed: 0,
    });
}

// Cap verification budget.
let MAX_V = 16;
let to_verify = [];
let i = 0;
for f in all_findings {
    if i < MAX_V {
        to_verify.push(f);
    }
    i += 1;
}
if all_findings.len() > MAX_V {
    log("truncated: verifying first " + MAX_V.to_string() + " of " + all_findings.len().to_string());
}

phase("Verify");

let vjobs = [];
for f in to_verify {
    let vp = "";
    vp += "Adversarially verify this documentation/comment drift finding against the harbor-kernel tree.\n";
    vp += "AUTHORITY: " + truth + "\n";
    vp += "CLAIMED PATH: " + f.path + "\n";
    vp += "ISSUE: " + f.issue + "\n";
    vp += "SUGGESTED FIX: " + f.suggested_fix + "\n";
    vp += "Use read_file/grep to inspect the file. Set real=true ONLY if the stale text still exists ";
    vp += "and contradicts the authority above. Prefer real=false for historical notes, accurate residuals, ";
    vp += "or already-fixed lines. evidence must quote the stale snippet or say not found.";
    vjobs.push(#{
        prompt: vp,
        label: "verify",
        capability_mode: "read-only",
        output_schema: verdict_schema,
    });
}

let verdicts = parallel(vjobs);

let confirmed = [];
let vi = 0;
for v in verdicts {
    if v != () && v.success && v.output.real == true
        && v.output.evidence != () && v.output.evidence != "" {
        confirmed.push(to_verify[vi]);
    }
    vi += 1;
}

log(confirmed.len().to_string() + "/" + to_verify.len().to_string() + " findings confirmed");

if confirmed.len() == 0 {
    let report = "# Doc/comment drift report\n\n";
    report += "Raw findings: " + all_findings.len().to_string() + "\n";
    report += "Confirmed after adversarial verify: 0\n";
    report += "No fixes applied.\n";
    let path = write_scratch_file("drift-report.md", report);
    complete(#{
        summary: "No confirmed drift after verification",
        path: path,
        confirmed: 0,
        fixed: 0,
    });
}

phase("Fix");

// Build a single fixer prompt with all confirmed items (bounded).
let fix_prompt = "";
fix_prompt += "You are fixing confirmed documentation/comment drift in harbor-kernel.\n";
fix_prompt += truth;
fix_prompt += "Apply MINIMAL edits only to the listed paths. English prose. Do not change runtime logic ";
fix_prompt += "except comment/doc strings. Do not invent HW stamps that do not exist.\n";
fix_prompt += "CONFIRMED FINDINGS TO FIX:\n";
let fi = 0;
for f in confirmed {
    fix_prompt += (fi + 1).to_string() + ") " + f.path + " — " + f.issue + "\n";
    fix_prompt += "   Fix: " + f.suggested_fix + "\n";
    fi += 1;
}
fix_prompt += "After edits, run: make xrefs doc-claims doc-symbols roadmap-evidence\n";
fix_prompt += "Return fixed[] paths actually changed, skipped[] with reasons, and summary.";

let fix_result = agent(fix_prompt, #{
    label: "fix:apply",
    capability_mode: "all",
    output_schema: fix_schema,
});

let fixed_n = 0;
let summary = "fixer failed";
let fixed_list = [];
let skipped_list = [];
if fix_result != () && fix_result.success && fix_result.output != () {
    summary = fix_result.output.summary;
    if fix_result.output.fixed != () {
        fixed_list = fix_result.output.fixed;
        fixed_n = fixed_list.len();
    }
    if fix_result.output.skipped != () {
        skipped_list = fix_result.output.skipped;
    }
}

let report = "# Doc/comment drift report\n\n";
report += "## Authority snapshot\n";
report += truth + "\n";
report += "## Counts\n";
report += "- Raw scan findings: " + all_findings.len().to_string() + "\n";
report += "- Verified window: " + to_verify.len().to_string() + "\n";
report += "- Confirmed: " + confirmed.len().to_string() + "\n";
report += "- Fixed entries reported: " + fixed_n.to_string() + "\n\n";
report += "## Confirmed findings\n";
for f in confirmed {
    report += "- `" + f.path + "`: " + f.issue + "\n";
}
report += "\n## Fixer summary\n";
report += summary + "\n\n";
report += "### Fixed\n";
for x in fixed_list {
    report += "- " + x + "\n";
}
report += "\n### Skipped\n";
for x in skipped_list {
    report += "- " + x + "\n";
}

let path = write_scratch_file("drift-report.md", report);
complete(#{
    summary: summary,
    path: path,
    confirmed: confirmed.len(),
    fixed: fixed_n,
});
