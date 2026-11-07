import type { Entity, Job, Page, Plan, Snapshot, Overview } from './types';
const MB = 1024 ** 2;
const row = (id: string, name: string, agent: string, bytes: number, category: string, sessions = 0, note = ''): Entity => ({ id, name, agent, bytes, allocated: bytes, category, sessions, path: category === 'associated' ? `source/projects/${id}` : `source/agents/${agent}/${category === 'exclusive' ? `sessions/${id}.jsonl` : 'cache'}`, updated: '2026-11-06 10:30 · 模拟', note: note || (category === 'associated' ? '会话元数据关联；项目空间只计算一次，不推断文件由哪个对话生成' : '只计明确归属文件；共享缓存与项目文件不重复分配') });
const sessions = [row('c1', '修复 Atlas 登录状态', 'codex', 90 * MB, 'exclusive', 0, '关联项目：atlas；含独占附件，不分摊共享数据库'), row('c2', '整理 API 错误处理', 'codex', 12 * MB, 'exclusive', 0, '关联项目：atlas；仅会话文件'), row('c3', '优化 Orbit 索引性能', 'codex', 24 * MB, 'exclusive', 0, '关联项目：orbit；仅会话文件'), row('h1', '设计 Atlas 设置页面', 'claude', 132 * MB, 'exclusive', 0, '关联项目：atlas；含独占设计附件'), row('h2', '更新个人知识库', 'claude', 6 * MB, 'exclusive', 0, '关联项目：notes；仅会话文件')];
const projects = [row('atlas', 'Atlas 工作台', 'workspace', 1240 * MB, 'associated', 3), row('orbit', 'Orbit 索引服务', 'workspace', 480 * MB, 'associated', 1), row('notes', '个人知识库', 'workspace', 64 * MB, 'associated', 1)];
const agents = [row('codex', 'Codex', 'codex', 638 * MB, 'shared', 3), row('claude', 'Claude Code', 'claude', 458 * MB, 'shared', 2)];
let jobs: Job[] = [];
try { jobs = JSON.parse(localStorage.getItem('agent-space-preview-jobs') || '[]'); } catch { /* disposable preview */ }
const save = () => { localStorage.setItem('agent-space-preview-jobs', JSON.stringify(jobs)); };
export function demoSnapshot(): Snapshot {
  return { mode: 'browser-preview', root: '浏览器模拟 · 不访问任何本地文件', scannedAt: String(Math.floor(Date.now() / 1000)), totalBytes: 2912 * MB, allocatedBytes: 2912 * MB, fileCount: 18342, exclusiveBytes: 264 * MB, sharedBytes: 832 * MB, projectBytes: 1784 * MB, unknownBytes: 32 * MB, backupBytes: jobs.filter(j => j.status === 'completed').reduce((n,j)=>n+j.bytes,0), agents, projects, sessions, jobs: [...jobs], warnings: [],volumes:[{volume:'C: · 模拟',bytes:2454*MB,allocatedBytes:2454*MB,files:16000},{volume:'D: · 模拟',bytes:458*MB,allocatedBytes:458*MB,files:2342}] };
}
export function demoPage(kind: 'agents' | 'projects' | 'sessions', search: string, offset: number, limit: number,sort='bytes',descending=true): Page { const all = demoSnapshot()[kind].filter(x => `${x.name} ${x.agent} ${x.path}`.toLowerCase().includes(search.toLowerCase())).sort((a,b)=>{const key=sort as keyof Entity;const x=a[key],y=b[key];return (typeof x==='number'&&typeof y==='number'?x-y:String(x).localeCompare(String(y),'zh-CN'))*(descending?-1:1)}); return { items: all.slice(offset, offset+limit), total: all.length }; }
export function demoOverview(volume:string):Overview {
  const s=demoSnapshot();
  // Explicitly synthetic partitions; both disks sum to the complete demo snapshot.
  const d=volume.startsWith('D:');const c=volume.startsWith('C:');
  const selectedAgents=d?[agents[1]]:c?[agents[0]]:agents;
  return {...s,selectedVolume:volume,agents:selectedAgents,agentFiles:{codex:56,claude:56},agentSubtotal:selectedAgents.reduce((n,a)=>n+a.bytes,0),
    totalBytes:d?458*MB:c?2454*MB:s.totalBytes,allocatedBytes:d?458*MB:c?2454*MB:s.allocatedBytes,
    fileCount:d?2342:c?16000:s.fileCount,sharedBytes:d?320*MB:c?512*MB:s.sharedBytes,
    exclusiveBytes:d?138*MB:c?126*MB:s.exclusiveBytes,projectBytes:d?0:s.projectBytes,unknownBytes:d?0:s.unknownBytes,
    projectCount:d?0:3,sessionCount:d?2:c?3:5};
}
export function demoPlan(agent: string): Plan {
  if (!['codex','claude'].includes(agent)) throw new Error('未知模拟适配器');
  if (jobs.some(j=>j.agent===agent&&j.status==='completed')) throw new Error('该模拟缓存已迁移，请先在迁移记录中恢复');
  return { agent, source: `模拟/source/agents/${agent}/cache`, destination: `模拟/destination/${agent}/cache`, bytes: (agent==='codex'?320:192)*MB, fileCount: 12, steps: ['检查项目测试区边界','复制并逐文件 SHA-256 校验','保留原副本，建立目录 Junction','验证入口并记录恢复信息'], warning: '浏览器仅模拟操作。桌面样本迁移保留副本，不释放空间。' };
}
export function demoExecute(agent: string): Job { const p = demoPlan(agent); const j: Job = { id: crypto.randomUUID(), agent, source:p.source, destination:p.destination, backup:'模拟/backups/保留原副本', bytes:p.bytes, status:'completed',createdAt:String(Math.floor(Date.now()/1000)),message:'模拟完成，释放 0 B',hashes:[] }; jobs.unshift(j); save(); return j; }
export function demoRollback(id: string): Job { const j = jobs.find(j=>j.id===id); if (!j) throw new Error('任务不存在'); j.status='rolledBack'; j.message='模拟恢复完成'; save(); return j; }
