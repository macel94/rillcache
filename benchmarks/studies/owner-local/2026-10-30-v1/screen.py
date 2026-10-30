#!/usr/bin/env python3
"""Advisory matched one-CPU/default-runtime A/B; not a protected performance gate."""
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import statistics
import subprocess

ROOT = Path.cwd()
OUT = ROOT / 'target/storage-ownership/owner-local-v1'
spec = importlib.util.spec_from_file_location('matched', ROOT / 'scripts/compare-builds.py')
matched = importlib.util.module_from_spec(spec)
spec.loader.exec_module(matched)

def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def git(*args): return subprocess.check_output(['git', *args])

baseline = ROOT / 'target/storage-ownership/rillcache-lock-backed-f0d7'
candidate = OUT / 'rillcache'
client = OUT / 'rillcache-bench'
patch = git('diff', '--binary', 'HEAD')
untracked = [p for p in git('ls-files', '--others', '--exclude-standard').decode().splitlines() if p.startswith(('src/', 'docs/'))]
for path in untracked:
    result = subprocess.run(['git','diff','--binary','--no-index','--','/dev/null',path], capture_output=True)
    assert result.returncode in (0,1), result.stderr
    patch += result.stdout
(OUT/'candidate.patch').write_bytes(patch)
source = {'base_sha':git('rev-parse','HEAD').decode().strip(),
          'tracked_diff_sha256':hashlib.sha256(git('diff','--binary','HEAD')).hexdigest(),
          'untracked_files_sha256':{p:sha(p) for p in untracked},
          'combined_patch_sha256':sha(OUT/'candidate.patch')}
cpus = sorted(os.sched_getaffinity(0))
assert len(cpus)>=2
hashes={n:sha(p) for n,p in [('before',baseline),('after',candidate),('client',client)]}
metadata={'schema_version':1,'complete':False,'kind':'advisory-default-one-cpu-ab',
    'created':datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'baseline_source':{'commit':'f0d7d186f01e74323cb7dfee19a99829de13b3af','diff_sha256':hashlib.sha256(b'').hexdigest(),'provenance':'lock-backed release binary frozen before the rejected lease build; see baseline-binaries.sha256'},
    'candidate_source':source,'binary_sha256':hashes,
    'allowed_cpus':cpus,'server_cpus':[cpus[0]],'client_cpus':cpus[1:], 'cpu_overlap':False,
    'hardware_sha256':sha(OUT/'hardware.json'),'rustc':subprocess.check_output(['rustc','-vV']).decode(),
    'build':'cargo build --locked --offline --release --bins, normal checked-in release profile',
    'runs':3,'requests':{'1':20000,'16':100000},'instrumentation':'none; ordinary client procfs accounting only',
    'scope':'default Unix plain TCP, one logical server CPU, 8 clients, 1024 keys, 64-byte values, 1000 warmups/client; shared agent host, not isolated performance hardware; no Valkey/multi-core claim',
    'measurement_order':[]}
(OUT/'manifest.json').write_text(json.dumps(metadata,indent=2)+'\n')
samples=[]
for pipeline in (1,16):
    for run in range(1,4):
        for variant in (('before','after') if run%2 else ('after','before')):
            assert {n:sha(p) for n,p in [('before',baseline),('after',candidate),('client',client)]}==hashes
            name=f'one-cpu-default-pipeline-{pipeline}-run-{run}-{variant}'
            binary=baseline if variant=='before' else candidate
            records=matched.measure(binary,client,0,'one-cpu',metadata['requests'][str(pipeline)],pipeline,cpus,OUT/name)
            metadata['measurement_order'].append(name)
            samples.extend((pipeline,variant,r) for r in records)
            print(name,'validated',flush=True)
            (OUT/'manifest.json').write_text(json.dumps(metadata,indent=2)+'\n')
assert len(samples)==60
text=['# Owner-local default-runtime A/B (advisory)', '', metadata['scope']+'.', '',
    'Three fresh alternating repetitions per build/pipeline; one owned server at a time, all replies validated. No concurrent task-local build/test ran during timing. Baseline and candidate use the same frozen client. Raw records retain batch tails and sampled CPU/accounting windows. One logical CPU does not establish physical-core isolation; host activity/frequency is uncontrolled. This is not either protected Valkey gate, a universal win, or scaling evidence.', '',
    f"Baseline: `{metadata['baseline_source']['commit']}` (clean diff). Candidate: base `{source['base_sha']}` plus combined patch SHA256 `{source['combined_patch_sha256']}`. See manifest and candidate.patch for tracked/untracked source attribution. Binary hashes are frozen in manifest.", '',
    '| P | Workload | Before cmd/s median [min–max] | After cmd/s median [min–max] | Delta | Before/after batch p99 µs | Before/after CPU µs/cmd |',
    '| --: | -- | --: | --: | --: | --: | --: |']
for p in (1,16):
    for w in matched.WORKLOADS:
        groups=[]
        for v in ('before','after'):
            rows=[r for pp,vv,r in samples if (pp,vv,r['workload'])==(p,v,w)]
            assert len(rows)==3
            rate=[r['ops_per_second'] for r in rows]
            cpu=[(r['server_profile']['user_ticks']+r['server_profile']['system_ticks'])/r['server_profile']['clock_ticks_per_second']/r['requests']*1e6 for r in rows]
            groups.append((statistics.median(rate),min(rate),max(rate),statistics.median(r['batch_p99_us'] for r in rows),statistics.median(cpu)))
        a,b=groups
        text.append(f'| {p} | {w} | {a[0]:.0f} [{a[1]:.0f}–{a[2]:.0f}] | {b[0]:.0f} [{b[1]:.0f}–{b[2]:.0f}] | {(b[0]/a[0]-1)*100:+.1f}% | {a[3]:.1f} / {b[3]:.1f} | {a[4]:.2f} / {b[4]:.2f} |')
text+=['','Whole-batch p99 is not per-command latency. CPU is tick-quantized, not an energy measure. No rows are omitted, including losses. External/multi-worker compatibility mode is outside this local-path cohort and pays synchronous routing/copy costs; release CI smoke/core evidence is retained separately, not compared to this cohort.']
(OUT/'summary.md').write_text('\n'.join(text)+'\n')
metadata['complete']=True
metadata['records']=len(samples)
metadata['evidence_sha256']={p.name:sha(p) for p in sorted(OUT.glob('*.json')) if p.name!='manifest.json'}
metadata['summary_sha256']=sha(OUT/'summary.md')
(OUT/'manifest.json').write_text(json.dumps(metadata,indent=2)+'\n')
print('complete; all ten A/B rows retained',flush=True)
