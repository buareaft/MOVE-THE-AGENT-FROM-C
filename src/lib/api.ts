import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import { demoExecute, demoPage, demoPlan, demoRollback, demoSnapshot, demoOverview } from './demo';
import type { Breakdown, Job, Kind, Page, Plan, Progress, Snapshot, RuntimeInfo, MoveCandidate, MoveJob, MovePlan, MoveProgress, Overview, StorageSettings, VirtualDiskStatus } from './types';
export const native = isTauri();
let cancelled = false;
const delay = (ms: number) => new Promise(r=>setTimeout(r,ms));
let demoSettings:StorageSettings={backend:'directory',directory:'D:\\AgentData',imagePath:'D:\\MoveYourAgent\\AgentData.vhdx',capacityGb:64,driveLetter:'V',autoMount:false,diskId:'demo'};
export const api = {
  storageSettings:():Promise<StorageSettings>=>native?invoke('get_storage_settings'):Promise.resolve({...demoSettings}),
  saveStorageSettings:(settings:StorageSettings):Promise<StorageSettings>=>native?invoke('save_storage_settings',{settings}):Promise.resolve().then(()=>{demoSettings={...settings};return {...demoSettings}}),
  virtualDisk:(action:string):Promise<VirtualDiskStatus>=>native?invoke('virtual_disk',{action}):Promise.resolve({state:'demo',message:'浏览器演示不创建或挂载磁盘'}),
  configuredMoveTarget:():Promise<string>=>invoke('configured_move_target'),
  runtime:():Promise<RuntimeInfo>=>native?invoke('get_runtime_info'):Promise.resolve({windowsSupported:false,dataMode:'demo',message:'浏览器演示不访问本机数据'}),
  snapshot: (): Promise<Snapshot> => native ? invoke('get_snapshot') : Promise.resolve(demoSnapshot()),
  overview:(volume:string):Promise<Overview>=>native?invoke('get_overview',{volume}):Promise.resolve(demoOverview(volume)),
  async scan(onProgress: (p:Progress)=>void):Promise<Snapshot> {
    if (native) { const channel = new Channel<Progress>(); channel.onmessage = onProgress; return invoke('start_scan', {onProgress:channel}); }
    cancelled = false;
    for (let n=1;n<=8;n++) { await delay(140); if(cancelled) throw new Error('扫描已取消，保留上次完整结果'); onProgress({phase:'正在演示索引流程',files:Math.round(18342*n/8),done:false}); }
    return demoSnapshot();
  },
  async cancel() { if(native) return invoke<void>('cancel_scan'); cancelled=true; },
  page: (kind:Kind,search:string,offset=0,limit=8,sort='bytes',descending=true):Promise<Page>=>native?invoke('list_entities',{kind,search,offset,limit,sort,descending}):Promise.resolve(demoPage(kind,search,offset,limit,sort,descending)),
  moveCandidates:():Promise<MoveCandidate[]>=>native?invoke('get_move_candidates'):Promise.resolve([]),
  moveJobs:():Promise<MoveJob[]>=>native?invoke('get_move_jobs'):Promise.resolve([]),
  prepareMove:(candidate:string,target:string):Promise<MovePlan>=>invoke('prepare_move',{candidate,target}),
  runMove:(id:string,action:'execute'|'restore'|'finalize',onProgress:(p:MoveProgress)=>void):Promise<MoveJob>=>{const channel=new Channel<MoveProgress>();channel.onmessage=onProgress;return invoke('run_move',{id,action,onProgress:channel})},
  cancelMove:():Promise<void>=>invoke('cancel_move'),
  openExport:(path:string):Promise<void>=>invoke('open_export',{path}),
  revealEntity:(kind:Kind,id:string):Promise<void>=>native?invoke('reveal_entity',{kind,id}):Promise.reject(new Error('浏览器演示无法打开本地目录，可复制示例路径')),
  async exportReport(format:'json'|'csv'):Promise<string>{
    if(native)return invoke('export_report',{format});
    const s=demoSnapshot();const text=format==='json'?JSON.stringify(s,null,2):'\uFEFFkind,name,agent,bytes\r\n'+(['agents','projects','sessions'] as const).flatMap(k=>s[k].map(e=>`${k},"${e.name.replaceAll('"','""')}",${e.agent},${e.bytes}`)).join('\r\n');
    const url=URL.createObjectURL(new Blob([text],{type:format==='json'?'application/json':'text/csv;charset=utf-8'}));const link=document.createElement('a');link.href=url;link.download=`agent-space-demo.${format}`;link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);return '浏览器演示报告已导出';
  },
  plan: (agent:string):Promise<Plan>=>native?invoke('create_migration_plan',{agent}):Promise.resolve().then(()=>demoPlan(agent)),
  execute: (agent:string):Promise<Job>=>native?invoke('execute_migration',{agent}):Promise.resolve().then(()=>demoExecute(agent)),
  rollback: (id:string):Promise<Job>=>native?invoke('rollback_migration',{id}):Promise.resolve().then(()=>demoRollback(id)),
  async breakdown(kind:Kind,id:string,volume=''):Promise<Breakdown[]> {
    if(native) return invoke('get_space_breakdown',{kind,id,volume});
    const row=(volume&&kind==='agents'?demoOverview(volume):demoSnapshot())[kind].find(x=>x.id===id); if(!row) return [];
    const labels=kind==='projects'?['依赖文件','构建产物','项目内容']:kind==='sessions'?['会话记录','附件']:['共享缓存','独占会话与附件'];
    const fractions=kind==='projects'?[0.64,0.26,0.1]:[0.7,0.3];
    const first=Math.round(row.bytes*fractions[0]); const second=kind==='projects'?Math.round(row.bytes*fractions[1]):row.bytes-first;
    return labels.map((label,i)=>({label,bytes:i===0?first:i===1?second:row.bytes-first-second,files:[44,12,28][i]}));
  }
};
