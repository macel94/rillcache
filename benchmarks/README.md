# Current benchmark performance

The comparative benchmark workflow now publishes validated performance tables and
an optional **GPT-6 Luna review through GitHub Copilot** here automatically.

The first automated report has not been published yet. Until it is, the
[last independently reviewed cohort](ci/10d56ec/README.md) and
[performance review](../docs/ci-benchmark-review.md) describe the verified baseline.

Each full comparison run also gets a permanent Markdown report under `reports/`
and an Actions job summary. Artifact downloads are only needed for raw evidence,
not for reading current performance. Short CI smoke timings are correctness checks
and do not replace the comparative report.

[Reporting and Copilot setup](../docs/benchmark-reports.md).
