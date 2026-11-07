import {useEffect,useState} from 'react';
import {HardDrive,LoaderCircle,RefreshCw} from 'lucide-react';
import {api,native} from '../lib/api';
import {formatBytes,type StorageSettings,type VirtualDiskStatus} from '../lib/types';
import {Button} from './ui/button';
import {Dialog} from './ui/dialog';
const errorText=(e:unknown)=>e instanceof Error?e.message:String(e);
export function Settings({localMode,onBusy}:{localMode:boolean;onBusy:(value:boolean)=>void}) {
  const [settings,setSettings]=useState<StorageSettings|null>(null),[saved,setSaved]=useState<StorageSettings|null>(null),[disk,setDisk]=useState<VirtualDiskStatus|null>(null);
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState(''),[confirm,setConfirm]=useState('');
  useEffect(()=>{let live=true;api.storageSettings().then(s=>{if(live){setSettings(s);setSaved(s)}}).catch(e=>{if(live)setError(errorText(e))});return()=>{live=false}},[]);
  useEffect(()=>{if(!saved||!localMode||saved.backend!=='vhdx')return;let live=true;api.virtualDisk('status').then(s=>{if(live)setDisk(s)}).catch(e=>{if(live)setError(errorText(e))});return()=>{live=false}},[saved,localMode]);
  function running(value:boolean){setBusy(value);onBusy(value)}
  async function save(){if(!settings)return;running(true);setError('');setNotice('');try{const s=await api.saveStorageSettings(settings);setSaved(s);setSettings(s);setNotice(native?'已保存':'已保存演示设置，本次页面有效')}catch(e){setError(errorText(e))}finally{running(false)}}
  async function action(action:string){setConfirm('');running(true);setError('');setNotice('');try{setDisk(await api.virtualDisk(action));const s=await api.storageSettings();setSaved(s);setSettings(s);setNotice(action==='status'?'状态已更新':'操作完成')}catch(e){setError(errorText(e))}finally{running(false)}}
  const dirty=JSON.stringify(settings)!==JSON.stringify(saved);
  const locked=!!disk&&disk.state!=='absent'&&disk.state!=='demo';
  const controlsDisabled=busy||dirty||!localMode;
  const actionLabels:Record<string,string>={create:'创建并挂载',mount:'挂载磁盘',detach:'卸载磁盘','enable-auto':'启用登录时挂载','disable-auto':'关闭登录时挂载'};
  if(!settings)return <div role="status">{error||'读取设置…'}</div>;
  return <div className="settings-page">
    {!localMode&&<p className="settings-note">{native?'测试区模式不执行磁盘操作。':'浏览器演示仅显示设置，不创建、格式化或挂载磁盘。'}</p>}
    <section className="settings-section"><div className="settings-heading"><h2>迁移存储</h2><p>迁移页默认使用此目标；已迁移目录不随设置自动改变。</p></div>
      <label className="settings-field"><span>存储方式</span><select aria-label="存储方式" value={settings.backend} disabled={busy} onChange={e=>setSettings({...settings,backend:e.target.value as StorageSettings['backend']})}><option value="directory">普通目录 + 目录联接</option><option value="vhdx">虚拟磁盘 VHDX + 目录联接</option></select></label>
      {settings.backend==='directory'?<label className="settings-field"><span>默认目标目录</span><input aria-label="默认目标目录" value={settings.directory} disabled={busy} onChange={e=>setSettings({...settings,directory:e.target.value})}/><small>迁移前需创建目标目录。</small></label>:<>
        <label className="settings-field"><span>镜像文件</span><input aria-label="镜像文件" value={settings.imagePath} disabled={busy||locked} onChange={e=>setSettings({...settings,imagePath:e.target.value})}/><small>选择 D 盘等非系统盘。实际文件随数据增长，最大不超过虚拟容量。</small></label>
        <div className="settings-inline"><label className="settings-field"><span>虚拟容量（GB）</span><input aria-label="虚拟容量" type="number" min={1} max={2048} value={settings.capacityGb} disabled={busy||locked} onChange={e=>setSettings({...settings,capacityGb:Number(e.target.value)})}/></label><label className="settings-field"><span>挂载盘符</span><select aria-label="挂载盘符" value={settings.driveLetter} disabled={busy||locked} onChange={e=>setSettings({...settings,driveLetter:e.target.value})}>{'DEFGHIJKLMNOPQRSTUVWXYZ'.split('').map(l=><option key={l} value={l}>{l}:</option>)}</select></label></div>
        {settings.imagePath.toUpperCase().startsWith('C:')&&<p className="settings-note">镜像位于 C 盘，迁入此镜像不会释放 C 盘的实际空间。</p>}
      </>}
      <div className="settings-actions"><Button disabled={busy||!dirty} onClick={save}>保存设置</Button><span role="status">{dirty?'有未保存的修改':notice}</span></div>
    </section>
    {settings.backend==='vhdx'&&<section className="settings-section"><div className="settings-heading"><h2><HardDrive size={16}/>虚拟磁盘</h2><Button variant="ghost" size="sm" disabled={controlsDisabled} onClick={()=>void action('status')}><RefreshCw size={14}/>刷新</Button></div>
      <dl className="disk-properties"><div><dt>状态</dt><dd>{disk?.message??(localMode?'正在读取…':'未连接本机磁盘')}</dd></div><div><dt>镜像位置</dt><dd>{saved?.imagePath}</dd></div><div><dt>迁移目标</dt><dd>{disk?.target||`${settings.driveLetter}:\\AgentData`}</dd></div>{disk?.state==='mounted'&&<><div><dt>卷可用 / 容量</dt><dd>{formatBytes(disk.free??0)} / {formatBytes(disk.size??0)}</dd></div><div><dt>镜像所在盘可用</dt><dd>{formatBytes(disk.backingFree??0)}</dd></div><div><dt>镜像文件大小</dt><dd>{formatBytes(disk.imageBytes??0)}</dd></div></>}<div><dt>登录时自动挂载</dt><dd>{disk?.autoMount?'已启用':'未启用'}</dd></div></dl>
      <div className="settings-actions"><Button disabled={controlsDisabled||!disk||!['absent','incomplete'].includes(disk.state)} onClick={()=>setConfirm('create')}>创建并挂载</Button><Button variant="outline" disabled={controlsDisabled||disk?.state!=='detached'} onClick={()=>setConfirm('mount')}>挂载</Button><Button variant="outline" disabled={controlsDisabled||disk?.state!=='mounted'} onClick={()=>setConfirm('detach')}>卸载</Button><Button variant="outline" disabled={controlsDisabled||!disk||!['mounted','detached'].includes(disk.state)} onClick={()=>setConfirm(disk?.autoMount?'disable-auto':'enable-auto')}>{disk?.autoMount?'关闭自动挂载':'启用自动挂载'}</Button></div>
      <p className="settings-note">创建、挂载和登录任务需要 Windows 管理员授权。仅初始化新建且身份匹配的镜像，不格式化已有磁盘。存在迁移引用时禁止卸载。</p>
    </section>}
    <section className="settings-section"><div className="settings-heading"><h2>后续写入与恢复</h2></div><p className="settings-note">迁移完成后，原目录保留 NTFS 目录联接。Agent 后续在这些目录内生成的文件将写入目标，应用无需常驻。支持的目录以迁移页列表为准。</p>{settings.backend==='vhdx'&&<p className="settings-note">重启后须先挂载磁盘再启动 Agent。登录任务无法保证早于其他自启动程序；使用 Agent 前确认磁盘已挂载。镜像丢失或未挂载时，不在原路径创建替代目录。</p>}<p className="settings-note">“迁移记录 → 恢复原位置”会恢复目标上的最新数据。清理原副本需要单独验证；不提供镜像删除操作。</p></section>
    {busy&&<p className="settings-note" role="status"><LoaderCircle size={15} className="spin"/>正在处理，请完成可能出现的 Windows 授权提示…</p>}
    {error&&<div role="alert" className="error-banner">{error}</div>}
    <Dialog open={!!confirm} onOpenChange={open=>{if(!open)setConfirm('')}} title={actionLabels[confirm]??'磁盘操作'} description="">{confirm&&<><div className="plan-paths"><label>镜像<code>{saved?.imagePath}</code></label><label>挂载目标<code>{saved?.driveLetter}:\AgentData · {saved?.capacityGb} GB</code></label></div><p className="settings-note">{confirm==='create'?'创建动态 VHDX，仅格式化其中的新分区。现有镜像和被占用盘符会被拒绝。':confirm==='enable-auto'?'为当前 Windows 用户注册登录时挂载任务。请在启动 Agent 前检查挂载状态。':confirm==='detach'?'卸载前请退出使用此卷的程序。有迁移引用时将拒绝操作。':'此操作只处理上方登记的磁盘或挂载任务。'}</p><div className="dialog-actions"><Button variant="outline" onClick={()=>setConfirm('')}>取消</Button><Button onClick={()=>void action(confirm)}>确认{actionLabels[confirm]}</Button></div></>}</Dialog>
  </div>
}
