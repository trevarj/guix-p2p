# Contributing

Contributions are accepted under `GPL-3.0-or-later`.

This project is part of the GNU Guix ecosystem, where many contributors care
deeply about software freedom, provenance, and human maintainability. Some Guix
contributors object to LLM-generated contributions or to LLM systems generally.
Treat that concern as legitimate project context, not as a social obstacle to
work around.

By submitting a patch, you certify that:

- You have the right to submit the work under `GPL-3.0-or-later`.
- You reviewed and understand the submitted changes.
- You did not knowingly include code, text, data, secrets, or credentials that
  cannot be redistributed under this project's license.

## AI-Assisted Contributions

Useful issue reports and LLM-assisted pull requests with tests, reasoning, and
clear review notes are preferred.

Hand-writing code is welcome for trivial changes, careful design work, or pure
enjoyment, but it should not be treated as a moral requirement. It is unfair to
ask contributors to spend time hand-writing routine code when a reviewed
LLM-assisted workflow can produce a maintainable patch with clear reasoning and
verification.

LLM-assisted contributions still require extra scrutiny. The legal and licensing
status of generated code is not settled in all jurisdictions or all fact
patterns. In particular, project maintainers should not assume that a contributor
can automatically license purely LLM-generated output under the GPL merely
because they prompted a model to produce it.

If a contribution used an LLM to generate code, documentation, tests, commit
messages, or other project material, disclose that in the pull request or patch
cover letter. Include:

- Which tool or model was used, if known.
- Which parts of the patch were generated or substantially rewritten by the LLM.
- What human review, testing, and rewriting you performed.
- Whether any generated output was copied verbatim.

Contributors remain responsible for provenance, review, and correctness. Treat
LLM output as untrusted draft material: inspect it, test it, simplify it, and
make sure the submitted patch is understood and maintainable.

Do not submit large verbatim LLM outputs. Maintainers may reject AI-assisted
patches when:

- The generated portions are substantial and not clearly human-authored.
- The provenance is unclear.
- The patch resembles code, text, or data from an incompatible source.
- The contributor cannot explain or maintain the result.
- The maintainers are not comfortable that the work can be accepted under
  `GPL-3.0-or-later`.

Small uses such as asking for spelling fixes, command examples, search queries,
or review checklists are lower risk, but still require human judgment.

This policy is not legal advice. It is a conservative project rule intended to
protect the GPL licensing status, contributor trust, and long-term maintainability
of the project.
