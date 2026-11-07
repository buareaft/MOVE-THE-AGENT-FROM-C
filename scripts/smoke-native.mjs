// Exercises the portable native core through its CLI. No user Agent discovery.
import { spawnSync } from 'node:child_process';
import { writeFile,copyFile,mkdir } from 'node:fs/promises';
import assert from 'node:assert/strict';
import path from 'node:path';

const sandbox=path.resolve('.lab/agent-space/native-smoke');
await mkdir(sandbox,{recursive:true});
await copyFile(path.resolve('artifacts/AgentSpace-Lab/agent-space-cli.exe'),path.join(sandbox,'agent-space-cli.exe'));
await writeFile(path.join(sandbox,'.agent-space-workspace'),'agent-space-fixture-workspace-v1');
const exe=path.join(sandbox,'agent-space-cli.exe');
function run(...args) {
  const p=spawnSync(exe,['--fixtures',...args],{cwd:path.dirname(exe),windowsHide:true,encoding:'utf8'});
  if(p.status!==0)throw new Error(p.stderr||p.error?.message||'Native CLI failed');
  return JSON.parse(p.stdout);
}
for(const job of run('jobs'))if(job.status!=='rolledBack')run('rollback',job.id);
const before=run('scan');
assert.equal(before.mode,'native-lab');
assert.equal(before.sessions.length,5);
const plan=run('plan','codex');
assert.ok(plan.source.includes('.lab'));
const job=run('execute','codex');
assert.equal(job.status,'completed');
const after=run('scan');
assert.equal(after.totalBytes,before.totalBytes);
assert.equal(after.sharedBytes,before.sharedBytes);
assert.equal(after.backupBytes,job.bytes);
assert.equal(run('rollback',job.id).status,'rolledBack');
const restored=run('scan');
assert.equal(restored.totalBytes,before.totalBytes);
assert.equal(restored.backupBytes,0);
await writeFile('artifacts/native-smoke.json',JSON.stringify({passed:true,executable:exe,totalBytes:restored.totalBytes,allocatedBytes:restored.allocatedBytes,checks:['portable runtime scope','fixture scan','cache plan','copy and hash verification','junction-aware scan without double count','rollback','rescan'],realAgentAccess:false},null,2));
console.log('Portable native smoke passed: fixture scan, junction migration, rollback, totals.');
