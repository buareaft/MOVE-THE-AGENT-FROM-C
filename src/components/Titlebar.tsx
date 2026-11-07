import {useEffect,useState} from 'react';
import {getCurrentWindow} from '@tauri-apps/api/window';
import {Copy,Layers3,Minus,Square,X} from 'lucide-react';
import {native} from '../lib/api';

export function Titlebar() {
  const [maximized,setMaximized]=useState(false),[error,setError]=useState('');
  useEffect(()=>{
    if(!native)return;
    const window=getCurrentWindow();let live=true;let stop:(()=>void)|undefined;
    const update=()=>window.isMaximized().then(value=>{if(live)setMaximized(value)}).catch(()=>{});
    void update();
    window.onResized(update).then(unlisten=>{if(live)stop=unlisten;else unlisten()}).catch(()=>{});
    return()=>{live=false;stop?.()};
  },[]);
  async function control(action:'minimize'|'maximize'|'close'){
    if(!native)return;setError('');
    try{const window=getCurrentWindow();if(action==='minimize')await window.minimize();else if(action==='close')await window.close();else{await window.toggleMaximize();setMaximized(await window.isMaximized())}}
    catch(e){setError(String(e))}
  }
  return <><header className="window-titlebar" aria-label="窗口标题栏"><div className="window-drag-region" data-tauri-drag-region><Layers3 size={14}/><span>Move Your Agent</span></div><div className="window-controls"><button aria-label="最小化窗口" title="最小化" disabled={!native} onClick={()=>void control('minimize')}><Minus size={15}/></button><button aria-label={maximized?'还原窗口':'最大化窗口'} title={maximized?'还原':'最大化'} disabled={!native} onClick={()=>void control('maximize')}>{maximized?<Copy size={12}/>:<Square size={12}/>}</button><button className="window-close" aria-label="关闭窗口" title="关闭" disabled={!native} onClick={()=>void control('close')}><X size={16}/></button></div></header>{error&&<div className="window-error" role="alert">{error}<button aria-label="关闭窗口提示" onClick={()=>setError('')}><X size={14}/></button></div>}</>;
}
