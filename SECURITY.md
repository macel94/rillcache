# Security policy

## Report a vulnerability privately

Do not report suspected vulnerabilities in a public issue, discussion, or pull request. Use GitHub's [private vulnerability reporting form](https://github.com/macel94/rillcache/security/advisories/new). Private vulnerability reporting is enabled for this repository. If the form is unavailable, do not publish exploit details; contact the repository owner through the private contact route listed on the GitHub profile and ask for a secure reporting channel.

Include the affected commit or image digest, deployment mode, impact, and a minimal reproduction. Redact credentials, customer data, and live secrets. Do not attach production data.

## Triage and disclosure

The maintainer targets an initial acknowledgement within five business days and an initial scope/severity assessment within ten business days. These are best-effort targets, not an SLA; there is no 24/7 response or guaranteed release/support window. If the report is accepted, keep coordination in the private advisory, agree on a disclosure date with the reporter, and provide status at least every ten business days while work is active. Publish a GitHub Security Advisory and corrected release/image digest when a fix or mitigation is available; credit the reporter only with their consent. If the report is out of scope or cannot be reproduced, explain the decision privately.

The project has not declared a stable supported-version policy. Until it does, triage starts with the current default-branch source and published container digests; this does not promise fixes for every historical image. Security fixes still use the protected PR and release workflow. Do not use a private source patch, bypass, or unsigned image as a security release.

## Limits

Dependabot security updates and secret scanning are enabled, but this is not a promise of complete dependency, license, or container vulnerability coverage. A generated SBOM is inventory evidence, not a vulnerability scan or security review. No independent security review or production acceptance is implied by this policy or by passing CI.
