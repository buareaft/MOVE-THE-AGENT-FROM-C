import {useCallback,useEffect,useState} from 'react';
import {ArrowDown,ArrowDownUp,CheckCheck,Folder,LoaderCircle,ShieldCheck,Undo2,X} from 'lucide-react';
import {api} from '../lib/api';
import {formatBytes,type MoveCandidate,type MoveJob,type MovePlan,type MoveProgress} from '../lib/types';
import {Button} from './ui/button';
import {Dialog} from './ui/dialog';
const errorText=(e:unknown)=>e instanceof Error?e.message:String(e);
const label=(job:MoveJob)=>job.status==='paused'?'已暂停':({planned:'待确认',copying:'复制中',copied:'副本已验证',renaming:'切换中',linking:'切换中',active:'已迁移 · 保留副本',purging:'清理待继续',committed:'已清理原副本',restoreCopy:'恢复复制中',restoreSwitch:'恢复切换中',restoreCleanup:'恢复待清理',restored:'已恢复',cancelled:'已取消'}[job.phase]??job.phase);
export function RealMigrations({onBusy,onChanged}:{onBusy:(b:boolean)=>void;onChanged:()=>Promise<void>}){
  const [candidates,setCandidates]=useState<MoveCandidate[]>([]);const [jobs,setJobs]=useState<MoveJob[]>([]);const [target,setTarget]=useState('');const [plan,setPlan]=useState<MovePlan|null>(null);const [confirm,setConfirm]=useState<{job:MoveJob;action:'restore'|'finalize'}|null>(null);const [busy,setBusy]=useState(false);const [error,setError]=useState('');const [progress,setProgress]=useState<MoveProgress|null>(null);const [cancelled,setCancelled]=useState(false);
  const reload=useCallback(async()=>{const [c,j]=await Promise.all([api.moveCandidates(),api.moveJobs()]);setCandidates(c);setJobs(j.filter(x=>x.candidate!=='diagnostic'));},[]);
  useEffect(()=>{reload().catch(e=>setError(errorText(e)));api.configuredMoveTarget().then(setTarget).catch(e=>setError(errorText(e)))},[reload]);
  function running(value:boolean){setBusy(value);onBusy(value);}
  async function prepare(candidate:MoveCandidate){running(true);setError('');try{setPlan(await api.prepareMove(candidate.id,target.trim()));await reload()}catch(e){setError(errorText(e))}finally{running(false)}}
  async function closePlan(){if(!plan||busy)return;const id=plan.id;setPlan(null);try{await api.runMove(id,'restore',()=>{});await reload()}catch(e){setError(errorText(e))}}
  async function run(id:string,action:'execute'|'restore'|'finalize'){
    running(true);setError('');setProgress({phase:action==='execute'?'检查与校验':action==='restore'?'检查恢复空间':'验证原副本',files:0,bytes:0,totalBytes:0});setCancelled(false);setPlan(null);setConfirm(null);
    try{await api.runMove(id,action,setProgress)}catch(e){setError(errorText(e))}finally{setProgress(null);running(false);await reload().catch(e=>setError(errorText(e)));await onChanged().catch(e=>setError(errorText(e)));}
  }
  const retained=jobs.reduce((n,j)=>n+j.retainedBytes,0);const freed=jobs.reduce((n,j)=>n+j.freedBytes,0);
  return <>
    <section className="panel real-migrations"><div className="panel-heading"><div><h2>目录迁移</h2><p>默认使用设置中的目标。目录联接将后续写入持续导向新位置。操作前退出 Agent。</p></div><ShieldCheck size={20}/></div>
      <div className="target-picker"><Folder size={18}/><label htmlFor="migration-target">目标根目录</label><input id="migration-target" aria-label="迁移目标根目录" placeholder="例如 E:\AgentSpaceData（请先创建目录）" value={target} onChange={e=>setTarget(e.target.value)} disabled={busy}/></div>
      <div className="migration-options">{!candidates.length?<p className="muted">未发现可迁移的已知目录。</p>:candidates.map(c=><div key={c.id}><span className="agent-icon"><Folder size={21}/></span><div><strong>{c.label}</strong><p title={c.source}>{c.reason||c.source}</p></div><Button variant="outline" size="sm" disabled={busy||!target.trim()||!c.available} onClick={()=>prepare(c)}>预览方案</Button></div>)}</div>
    </section>
    {error&&<div className="error-banner" role="alert"><ShieldCheck size={18}/><span>{error}</span><button aria-label="关闭迁移提示" onClick={()=>setError('')}><X size={16}/></button></div>}
    {progress&&<div className="scan-banner" role="status"><LoaderCircle size={17} className="spin"/><span>{progress.phase} · {formatBytes(progress.bytes)}{progress.totalBytes?` / ${formatBytes(progress.totalBytes)}`:''} · {progress.files} 文件</span><Button size="sm" variant="outline" disabled={cancelled} onClick={()=>{setCancelled(true);api.cancelMove().catch(e=>setError(errorText(e)))}}>{cancelled?'正在取消':'取消任务'}</Button></div>}
    <section className="panel migration-history"><div className="panel-heading"><div><h2>迁移记录 <span className="count">{jobs.length}</span></h2><p>保留原副本 {formatBytes(retained)} · 跨盘清理的源文件占用 {formatBytes(freed)}</p></div><Button size="sm" variant="ghost" disabled={busy} onClick={()=>reload().catch(e=>setError(errorText(e)))}>刷新记录</Button></div>
      {!jobs.length?<div className="empty-state"><span><ArrowDownUp size={26}/></span><h3>暂无迁移记录</h3><p></p></div>:jobs.map(j=><div className="job-row" key={j.id}><span className="job-icon"><CheckCheck size={20}/></span><div className="move-job-main"><strong>{j.label} <span className="badge">{label(j)}</span></strong><p>{j.message}</p><code title={j.destination}>{j.destination}</code><small>{formatBytes(j.bytes)} · {new Date(Number(j.createdAt)*1000).toLocaleString('zh-CN')} · {j.id.slice(0,8)}</small></div>
        <div className="move-job-actions">{!['restored','cancelled'].includes(j.phase)&&<>
          {['planned','copying','copied','renaming','linking'].includes(j.phase)&&<Button size="sm" disabled={busy} onClick={()=>run(j.id,'execute')}>继续迁移</Button>}
          {['active','purging'].includes(j.phase)&&<Button size="sm" disabled={busy} onClick={()=>setConfirm({job:j,action:'finalize'})}>验证并清理副本</Button>}
          <Button variant="outline" size="sm" disabled={busy} onClick={()=>setConfirm({job:j,action:'restore'})}><Undo2 size={14}/>{j.phase.startsWith('restore')?'继续恢复':j.phase==='planned'?'取消方案':'恢复原位置'}</Button>
        </>}</div></div>)}
    </section>
    <Dialog open={!!plan} onOpenChange={open=>{if(!open)void closePlan()}} title="迁移方案" description="">{plan&&<><div className="plan-paths"><label>来源<code>{plan.source}</code></label><ArrowDown size={18}/><label>目标<code>{plan.destination}</code></label></div><div className="plan-summary"><span>文件大小<strong>{formatBytes(plan.bytes)}</strong></span><span>文件<strong>{plan.fileCount}</strong></span><span>磁盘<strong>{plan.crossVolume?'跨盘':'同盘'}</strong></span></div><div className="plan-warning"><ShieldCheck size={20}/><p>{plan.warning} {plan.crossVolume?`清理副本后可释放约 ${formatBytes(plan.allocatedBytes)}。`:'同盘迁移不会释放源盘空间。'}</p></div><div className="dialog-actions"><Button variant="outline" onClick={()=>void closePlan()}>取消方案</Button><Button onClick={()=>run(plan.id,'execute')}>确认并开始迁移</Button></div></>}</Dialog>
    <Dialog open={!!confirm} onOpenChange={open=>{if(!open)setConfirm(null)}} title={confirm?.action==='finalize'?'验证并清理原副本':'恢复原位置'} description={confirm?.job.label??''}>{confirm&&<><div className="plan-paths"><label>原位置<code>{confirm.job.source}</code></label><label>当前目标<code>{confirm.job.destination}</code></label></div><div className="plan-warning"><ShieldCheck size={20}/><p>{confirm.action==='finalize'?'重新校验目标和原副本后删除原副本；目标已更新则保留。请在重新使用 Agent 前清理。':'将目标上的最新数据恢复到原位置。源盘需有足够空间；校验失败则保留现有数据。'} 请先退出相关 Agent。</p></div><div className="dialog-actions"><Button variant="outline" onClick={()=>setConfirm(null)}>取消</Button><Button onClick={()=>run(confirm.job.id,confirm.action)}>确认{confirm.action==='finalize'?'清理':'恢复'}</Button></div></>}</Dialog>
  </>
}
