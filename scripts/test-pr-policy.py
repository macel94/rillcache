#!/usr/bin/env python3
"""Offline policy and report-PR tests. No network, credentials or real Git writes."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from pr_gate_policy import required_checks, required_workflows

ROOT = Path(__file__).resolve().parents[1]
FAKE = '''#!/usr/bin/env python3
import json, os, pathlib, sys
path=pathlib.Path(os.environ['FAKE_STATE'])
s=json.loads(path.read_text())
name=pathlib.Path(sys.argv[0]).name
args=sys.argv[1:]
s['calls'].append([name, args])
code=0; output=''
repo=os.environ.get('GITHUB_REPOSITORY','example/project')
branch='automation/benchmark-report-123-2'
head='a'*40; main='b'*40; blob='c'*40
if name == 'git':
 if args[:2] == ['switch','--create']: s['branch']=args[2]
 if 'commit' in args and s['branch'] == 'main': code=99
 if args[:1] == ['push'] and (s['branch'] == 'main' or args[-1] != 'HEAD:refs/heads/'+s['branch']): code=98
 if args == ['diff','--cached','--quiet']: code=0 if os.environ.get('NO_CHANGE') else 1
elif name == 'gh':
 if args[:3] == ['api','--method','GET']:
  endpoint=args[3]
  if endpoint.endswith('/pulls'):
   s['lists']+=1
   output='42' if os.environ.get('EXISTING_PR') or s['lists'] > 1 else ''
  elif endpoint.endswith('/pulls/42'):
   s['head_reads']=s.get('head_reads',0)+1
   sha='d'*40 if os.environ.get('HEAD_RACE') and s['head_reads']>1 else head
   merged=bool(os.environ.get('IMMEDIATE_MERGE') and s.get('merge_requested'))
   auto={'merge_method':'squash'} if s.get('merge_requested') and not os.environ.get('AUTO_UNVERIFIED') else None
   output=json.dumps({'number':42,'state':'closed' if merged else 'open','draft':bool(os.environ.get('PR_DRAFT')),
     'merged_at':'2026-10-26T00:00:00Z' if merged else None,'auto_merge':auto,
     'head':{'ref':branch,'sha':sha,'repo':{'full_name':repo}},'base':{'ref':'main','sha':main,'repo':{'full_name':repo}}})
  elif '/git/ref/heads/main' in endpoint:
   s['main_reads']=s.get('main_reads',0)+1
   output=json.dumps({'object':{'sha':'d'*40 if os.environ.get('MAIN_RACE') and s['main_reads']>1 else main}})
  elif endpoint.endswith('/rulesets/25863639'):
   policy=json.loads((pathlib.Path(os.environ['FAKE_PROJECT_ROOT'])/'.github/main-ruleset.json').read_text())
   if os.environ.get('LIVE_RULE_DRIFT'):
    checks=next(r['parameters']['required_status_checks'] for r in policy['rules'] if r['type']=='required_status_checks')
    checks[:]=[c for c in checks if c['context']!='Valkey 4-CPU performance gate']
   output=json.dumps(policy)
  elif endpoint.endswith('/rules/branches/main'):
   output=json.dumps(json.loads((pathlib.Path(os.environ['FAKE_PROJECT_ROOT'])/'.github/main-ruleset.json').read_text())['rules'])
  elif '/actions/workflows?per_page=100' in endpoint:
   workflows=json.loads((pathlib.Path(os.environ['FAKE_PROJECT_ROOT'])/'.github/pr-workflow-gates.json').read_text())['workflows']
   output=json.dumps({'total_count':len(workflows),'workflows':[{'path':'.github/workflows/'+p,'state':'active'} for p in workflows]})
  elif '/compare/' in endpoint: output=json.dumps({'status':'diverged' if os.environ.get('STALE') else 'ahead'})
  elif '/actions/workflows/' in endpoint and '/runs?' in endpoint:
   workflow=endpoint.split('/actions/workflows/',1)[1].split('/runs?',1)[0]
   mode=os.environ.get('RUN_MODE','success' if os.environ.get('EXISTING_PR') else 'missing')
   rows=[]
   if mode!='missing':
    status='in_progress' if mode=='running' else 'completed'
    conclusion=None if mode=='running' else 'action_required' if mode=='held' else 'failure' if 'failed' in mode else 'success'
    row={'id':100,'path':'.github/workflows/'+workflow,'head_branch':branch,'head_sha':head,
     'event':'pull_request' if mode=='held' or mode.startswith('pr_') else 'workflow_dispatch','status':status,'conclusion':conclusion,
     'head_repository':{'full_name':repo},'pull_requests':[] if mode.startswith('pr_') else [{'number':42}]}
    rows=[row]
    if mode=='failed_then_success': rows.append(dict(row,id=101,conclusion='success'))
   output=json.dumps({'workflow_runs':rows})
  elif '/contents/.github/workflows/' in endpoint:
   output=json.dumps({'sha':'d'*40 if os.environ.get('UNTRUSTED_WORKFLOW') and endpoint.endswith(head) else blob})
  else: code=95
  if os.environ.get('API_FAIL'): code=1
  if os.environ.get('INVALID_JSON'): output='not json'
 elif args[:2] == ['pr','create']: output='https://github.com/example/project/pull/42'
 elif args[:2] == ['pr','merge']:
  if os.environ.get('AUTO_FAIL'): code=1
  else: s['merge_requested']=True
  if '--admin' in args or '--auto' not in args or '--squash' not in args: code=96
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
        self.assertEqual(status['required_status_checks'], required_checks())
        self.assertEqual(len(status['required_status_checks']), 14)

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
        multicpu = (ROOT / '.github/workflows/pr-multicpu-quality.yml').read_text()
        for text in ('pull_request:', 'branches: [main]', 'name: Valkey 4-CPU performance gate',
                     'base/scripts/pr-multicpu-performance.py', 'valkey-benchmark',
                     '--generator', '--check evidence/gate', 'persist-credentials: false', 'if: always()'):
            self.assertIn(text, multicpu)
        for text in ('pull_request_target:', 'continue-on-error:', 'paths:', 'paths-ignore:', 'secrets.', 'contents: write'):
            self.assertNotIn(text, multicpu)
        self.assertIn('name: Tests and offline CI', (ROOT / '.github/workflows/ci.yml').read_text())
        owners = (ROOT / '.github/CODEOWNERS').read_text()
        for path in ('/.github/CODEOWNERS', '/.github/main-ruleset.json', '/.github/workflows/',
                     '/scripts/pr-performance-gate.py', '/scripts/compare-builds.py', '/benchmarks/targets.mk'):
            self.assertIn(path + ' @macel94', owners)

    def test_hosted_updater_has_a_read_only_pr_boundary_and_trusted_main_writer(self):
        workflow = (ROOT / '.github/workflows/pr-branch-updater.yml').read_text()
        self.assertIn('pull_request:', workflow)
        self.assertIn("if: github.event_name == 'pull_request'", workflow)
        self.assertIn("if: github.event_name != 'pull_request' && github.ref == 'refs/heads/main'", workflow)
        self.assertIn('workflow_run:', workflow)
        self.assertIn('workflow_dispatch:', workflow)
        self.assertIn('schedule:', workflow)
        self.assertIn('ref: main', workflow)
        self.assertIn('persist-credentials: false', workflow)
        self.assertIn('queue: max', workflow)
        self.assertIn('cancel-in-progress: false', workflow)
        self.assertNotIn('pull_request_target:', workflow)
        self.assertNotIn('secrets.', workflow)
        self.assertIn('contents: write', workflow)
        self.assertIn('pull-requests: write', workflow)
        self.assertIn('actions: write', workflow)
        helper = (ROOT / 'scripts/update-pr-branches.py').read_text()
        for value in ('ci.yml', 'pr-quality.yml', 'expected_head_sha', 'workflow_dispatch', 'github-actions'):
            self.assertIn(value, helper)
        self.assertNotIn('gh pr merge', helper)
        self.assertNotIn('git push', helper)

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
                       FAKE_STATE=str(state), FAKE_PROJECT_ROOT=str(ROOT), GITHUB_REPOSITORY='example/project', GITHUB_RUN_ID='123',
                       GITHUB_RUN_ATTEMPT='2', DEFAULT_BRANCH='main', RUNNER_TEMP=str(root),
                       GITHUB_STEP_SUMMARY=str(root / 'summary.md'))
            env.update(extra)
            result = subprocess.run(['bash', str(ROOT / 'scripts/publish-report-pr.sh')], cwd=root,
                                    env=env, capture_output=True, text=True, timeout=10)
            return result, json.loads(state.read_text())

    def test_new_report_commits_only_on_a_non_main_branch_dispatches_and_enables_verified_auto_merge(self):
        result, state = self.run_publisher()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(state['branch'], 'automation/benchmark-report-123-2')
        git = [args for name, args in state['calls'] if name == 'git']
        self.assertEqual([args for args in git if args[0] == 'push'], [['push', 'origin', 'HEAD:refs/heads/automation/benchmark-report-123-2']])
        self.assertTrue(any('commit' in args for args in git))
        gh = [args for name, args in state['calls'] if name == 'gh']
        self.assertTrue(any(args[:2] == ['pr', 'create'] for args in gh))
        dispatches = [args for args in gh if args[:2] == ['workflow', 'run']]
        self.assertEqual([args[2] for args in dispatches], list(required_workflows()))
        self.assertTrue(all(args[args.index('--ref') + 1] == state['branch'] for args in dispatches))
        self.assertIn('pull_request=42', dispatches[1])
        self.assertIn('pull_request=42', dispatches[2])
        merge = [args for args in gh if args[:2] == ['pr', 'merge']]
        self.assertEqual(len(merge), 1)
        self.assertIn('--auto', merge[0])
        self.assertIn('--squash', merge[0])
        self.assertNotIn('--admin', merge[0])
        self.assertEqual(merge[0][merge[0].index('--match-head-commit') + 1], 'a' * 40)
        self.assertIn('protected squash auto-merge verified', result.stdout)

    def test_noop_report_does_not_commit_push_or_open_a_pr(self):
        result, state = self.run_publisher(NO_CHANGE='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any('commit' in args or args[:1] == ['push'] for name, args in state['calls'] if name == 'git'))
        self.assertFalse(any(args[:2] in (['pr', 'create'], ['workflow', 'run']) for name, args in state['calls'] if name == 'gh'))

    def test_existing_pr_is_reused_without_rewriting_its_branch(self):
        result, state = self.run_publisher(EXISTING_PR='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(name == 'git' for name, _ in state['calls']))
        self.assertEqual(sum(args[:2] == ['workflow', 'run'] for name, args in state['calls'] if name == 'gh'), 0)
        self.assertFalse(any(args[:2] == ['pr', 'merge'] for name, args in state['calls'] if name == 'gh'))
        self.assertIn('preserving existing merge intent', result.stdout)

    def test_invalid_environment_is_rejected_before_git_or_api_work(self):
        for value in ({'DEFAULT_BRANCH': 'other'}, {'GITHUB_RUN_ID': '1;echo bad'}, {'GITHUB_REPOSITORY': 'bad/repo;echo bad'}):
            result, state = self.run_publisher(**value)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(state['calls'], [])

    def test_failed_check_dispatch_is_visible_instead_of_publication_success(self):
        result, _ = self.run_publisher(DISPATCH_FAIL='1')
        self.assertNotEqual(result.returncode, 0)

    def test_live_rule_drift_prevents_auto_merge_even_if_every_workflow_reports_success(self):
        result, state = self.run_publisher(LIVE_RULE_DRIFT='1', RUN_MODE='success')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('LIVE RULESET DRIFT', result.stderr)
        self.assertFalse(any(args[:2] in (['workflow', 'run'], ['pr', 'merge']) for name, args in state['calls'] if name == 'gh'))

    def test_existing_running_checks_are_not_duplicated(self):
        result, state = self.run_publisher(EXISTING_PR='1', RUN_MODE='running')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(args[:2] == ['workflow', 'run'] for name, args in state['calls'] if name == 'gh'))

    def test_any_exact_head_failure_is_preserved_even_after_a_later_success(self):
        for mode in ['failed', 'failed_then_success', 'pr_failed_no_association']:
            result, state = self.run_publisher(EXISTING_PR='1', RUN_MODE=mode)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('refusing a retry', result.stderr)
            self.assertFalse(any(args[:2] in (['workflow', 'run'], ['pr', 'merge']) for name, args in state['calls'] if name == 'gh'))

    def test_pr_runs_without_association_metadata_are_not_duplicated(self):
        result, state = self.run_publisher(EXISTING_PR='1', RUN_MODE='pr_no_association')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(args[:2] == ['workflow', 'run'] for name, args in state['calls'] if name == 'gh'))

    def test_approval_held_runs_do_not_masquerade_as_tests_or_prevent_explicit_dispatch(self):
        result, state = self.run_publisher(RUN_MODE='held')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sum(args[:2] == ['workflow', 'run'] for name, args in state['calls'] if name == 'gh'), 6)

    def test_untrusted_workflow_or_stale_or_changed_head_main_fail_before_dispatch(self):
        for flag in ['UNTRUSTED_WORKFLOW', 'STALE', 'HEAD_RACE', 'MAIN_RACE', 'PR_DRAFT', 'API_FAIL', 'INVALID_JSON']:
            result, state = self.run_publisher(**{flag: '1'})
            self.assertNotEqual(result.returncode, 0, flag)
            self.assertFalse(any(args[:2] in (['workflow', 'run'], ['pr', 'merge']) for name, args in state['calls'] if name == 'gh'), flag)

    def test_failed_or_unverified_auto_merge_is_not_publication_success(self):
        for flag in ['AUTO_FAIL', 'AUTO_UNVERIFIED']:
            result, _ = self.run_publisher(**{flag: '1'})
            self.assertNotEqual(result.returncode, 0, flag)
            self.assertNotIn('protected squash auto-merge verified', result.stdout)

    def test_normal_immediate_protected_merge_is_accepted(self):
        result, _ = self.run_publisher(IMMEDIATE_MERGE='1')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('normally merged through the protected PR path', result.stdout)


if __name__ == '__main__':
    unittest.main()
