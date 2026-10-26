#!/usr/bin/env python3
"""Offline policy and report-PR tests. No network, credentials or real Git writes."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
FAKE = '''#!/usr/bin/env python3
import json, os, pathlib, sys
path=pathlib.Path(os.environ['FAKE_STATE'])
s=json.loads(path.read_text())
name=pathlib.Path(sys.argv[0]).name
args=sys.argv[1:]
s['calls'].append([name, args])
code=0; output=''
if name == 'git':
 if args[:2] == ['switch','--create']: s['branch']=args[2]
 if 'commit' in args and s['branch'] == 'main': code=99
 if args[:1] == ['push'] and (s['branch'] == 'main' or args[-1] != 'HEAD:refs/heads/'+s['branch']): code=98
 if args == ['diff','--cached','--quiet']: code=0 if os.environ.get('NO_CHANGE') else 1
elif name == 'gh':
 if args[:3] == ['api','--method','GET']:
  s['lists']+=1
  output='42' if os.environ.get('EXISTING_PR') or s['lists'] > 1 else ''
 elif args[:2] == ['pr','create']: output='https://github.com/example/project/pull/42'
 elif args[:2] == ['workflow','run']:
  if os.environ.get('DISPATCH_FAIL'): code=1
  if '--ref' not in args or args[args.index('--ref')+1] == 'main': code=97
path.write_text(json.dumps(s))
print(output)
sys.exit(code)
'''


class PolicyTests(unittest.TestCase):
    def test_enforced_main_policy_has_no_bypass_or_ambiguous_required_context(self):
        policy = json.loads((ROOT / '.github/main-ruleset.json').read_text())
        self.assertEqual((policy['target'], policy['enforcement'], policy['bypass_actors']), ('branch', 'active', []))
        self.assertEqual(policy['conditions'], {'ref_name': {'include': ['refs/heads/main'], 'exclude': []}})
        rules = {rule['type']: rule.get('parameters', {}) for rule in policy['rules']}
        self.assertEqual(set(rules), {'deletion', 'non_fast_forward', 'required_linear_history', 'pull_request', 'required_status_checks'})
        self.assertEqual(rules['pull_request']['allowed_merge_methods'], ['squash'])
        self.assertEqual(rules['pull_request']['required_approving_review_count'], 0)
        self.assertIs(rules['pull_request']['require_code_owner_review'], True)
        self.assertIs(rules['pull_request']['required_review_thread_resolution'], True)
        status = rules['required_status_checks']
        self.assertIs(status['strict_required_status_checks_policy'], True)
        self.assertIs(status['do_not_enforce_on_create'], False)
        self.assertEqual(status['required_status_checks'], [
            {'context': 'Tests and offline CI', 'integration_id': 15368},
            {'context': 'Valkey performance gate', 'integration_id': 15368}])

    def test_every_pr_runs_read_only_frozen_tools_without_path_exemptions(self):
        workflow = (ROOT / '.github/workflows/pr-quality.yml').read_text()
        for text in ('pull_request:', 'branches: [main]', 'name: Valkey performance gate',
                     'persist-credentials: false', 'steps.context.outputs.base',
                     'base/scripts/pr-performance-gate.py', 'base/scripts/pr-performance-gate.py',
                     '--candidate-source "$GITHUB_SHA"', '--check evidence/gate', 'if: always()',
                     'run: make check', 'merge-base --is-ancestor', '--candidate', '--client', '--valkey'):
            self.assertIn(text, workflow)
        for text in ('pull_request_target:', 'continue-on-error:', 'paths:', 'paths-ignore:', 'secrets.', 'contents: write'):
            self.assertNotIn(text, workflow)
        self.assertIn('name: Tests and offline CI', (ROOT / '.github/workflows/ci.yml').read_text())
        owners = (ROOT / '.github/CODEOWNERS').read_text()
        for path in ('/.github/CODEOWNERS', '/.github/main-ruleset.json', '/.github/workflows/',
                     '/scripts/pr-performance-gate.py', '/scripts/compare-builds.py', '/benchmarks/targets.mk'):
            self.assertIn(path + ' @macel94', owners)

    def test_publisher_is_default_branch_only_and_cannot_push_main(self):
        workflow = (ROOT / '.github/workflows/benchmarks.yml').read_text().split('\n  publish:\n')[1]
        self.assertIn('github.ref == format', workflow)
        for permission in ('contents: write', 'pull-requests: write', 'actions: write'):
            self.assertIn(permission, workflow)
        self.assertIn('bash scripts/publish-report-pr.sh', workflow)
        self.assertNotIn('git push', workflow)
        self.assertNotIn('copilot-requests:', workflow)
        script = (ROOT / 'scripts/publish-report-pr.sh').read_text()
        self.assertIn('git switch --create "$branch"', script)
        self.assertIn('git push origin "HEAD:refs/heads/$branch"', script)
        self.assertNotIn('--force', script)
        self.assertNotIn('HEAD:$DEFAULT_BRANCH', script)
        self.assertIn('gh pr create', script)

    def run_publisher(self, **extra):
        with tempfile.TemporaryDirectory(prefix='rillcache-publisher-policy-') as name:
            root = Path(name)
            state = root / 'state.json'
            state.write_text(json.dumps(dict(branch='main', calls=[], lists=0)))
            for path in (root / 'bin/git', root / 'bin/gh', root / 'target/release/rillcache-review'):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(FAKE); path.chmod(0o700)
            env = dict(os.environ, PATH=str(root / 'bin') + os.pathsep + os.environ['PATH'],
                       FAKE_STATE=str(state), GITHUB_REPOSITORY='example/project', GITHUB_RUN_ID='123',
                       GITHUB_RUN_ATTEMPT='2', DEFAULT_BRANCH='main', RUNNER_TEMP=str(root),
                       GITHUB_STEP_SUMMARY=str(root / 'summary.md'))
            env.update(extra)
            result = subprocess.run(['bash', str(ROOT / 'scripts/publish-report-pr.sh')], cwd=root,
                                    env=env, capture_output=True, text=True, timeout=10)
            return result, json.loads(state.read_text())

    def test_new_report_commits_only_on_a_non_main_branch_and_dispatches_both_checks(self):
        result, state = self.run_publisher()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(state['branch'], 'automation/benchmark-report-123-2')
        git = [args for name, args in state['calls'] if name == 'git']
        self.assertEqual([args for args in git if args[0] == 'push'], [['push', 'origin', 'HEAD:refs/heads/automation/benchmark-report-123-2']])
        self.assertTrue(any('commit' in args for args in git))
        gh = [args for name, args in state['calls'] if name == 'gh']
        self.assertTrue(any(args[:2] == ['pr', 'create'] for args in gh))
        dispatches = [args for args in gh if args[:2] == ['workflow', 'run']]
        self.assertEqual([args[2] for args in dispatches], ['ci.yml', 'pr-quality.yml'])
        self.assertTrue(all(args[args.index('--ref') + 1] == state['branch'] for args in dispatches))
        self.assertIn('pull_request=42', dispatches[1])

    def test_noop_report_does_not_commit_push_or_open_a_pr(self):
        result, state = self.run_publisher(NO_CHANGE='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any('commit' in args or args[:1] == ['push'] for name, args in state['calls'] if name == 'git'))
        self.assertFalse(any(args[:2] in (['pr', 'create'], ['workflow', 'run']) for name, args in state['calls'] if name == 'gh'))

    def test_existing_pr_is_reused_without_rewriting_its_branch(self):
        result, state = self.run_publisher(EXISTING_PR='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(name == 'git' for name, _ in state['calls']))
        self.assertEqual(sum(args[:2] == ['workflow', 'run'] for name, args in state['calls'] if name == 'gh'), 2)

    def test_invalid_environment_is_rejected_before_git_or_api_work(self):
        for value in ({'DEFAULT_BRANCH': 'other'}, {'GITHUB_RUN_ID': '1;echo bad'}, {'GITHUB_REPOSITORY': 'bad/repo;echo bad'}):
            result, state = self.run_publisher(**value)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(state['calls'], [])

    def test_failed_check_dispatch_is_visible_instead_of_publication_success(self):
        result, _ = self.run_publisher(DISPATCH_FAIL='1')
        self.assertNotEqual(result.returncode, 0)


if __name__ == '__main__':
    unittest.main()
