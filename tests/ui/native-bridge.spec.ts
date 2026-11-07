import {test,expect} from '@playwright/test';

// Contract tests with synthetic replies. These do not perform native file operations.
test.beforeEach(async({page})=>{
  await page.addInitScript(()=>{
    const calls:{command:string;args:Record<string,unknown>}[]=[];
    const candidate={id:'codex-sessions',agent:'codex',label:'Codex 会话记录',source:'C:\\synthetic\\sessions',available:true,reason:''};
    let jobs:Record<string,unknown>[]=[];
    let storage={backend:'directory',directory:'D:\\AgentData',imagePath:'D:\\MoveYourAgent\\AgentData.vhdx',capacityGb:64,driveLetter:'V',autoMount:false,diskId:'synthetic-disk'};
    let diskState='absent';
    let maximized=false;
    const snapshot={mode:'native-local',root:'合成桥接测试',scannedAt:'1793977200',totalBytes:4194304,allocatedBytes:4194304,fileCount:2,exclusiveBytes:4194304,sharedBytes:0,projectBytes:0,unknownBytes:0,backupBytes:0,agents:[{id:'codex',name:'Codex',agent:'codex',path:'synthetic',bytes:4194304,allocated:4194304,category:'shared',sessions:42,updated:'',note:''}],projects:[],sessions:[],projectCount:1,sessionCount:42,jobs:[],warnings:[]};
    Object.assign(window,{isTauri:true,__testCalls:calls,__TAURI_INTERNALS__:{metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(command:string,args:Record<string,unknown>={})=>{
      calls.push({command,args});
      switch(command){
        case 'plugin:window|is_maximized':return maximized;
        case 'plugin:window|toggle_maximize':maximized=!maximized;return;
        case 'plugin:window|minimize':case 'plugin:window|close':case 'plugin:event|unlisten':return;
        case 'plugin:event|listen':return 1;
        case 'get_runtime_info':return {windowsSupported:true,dataMode:'local',message:'合成测试环境'};
        case 'get_storage_settings':return {...storage};
        case 'save_storage_settings':storage={...args.settings as typeof storage};return {...storage};
        case 'configured_move_target':return '';
        case 'virtual_disk':{if(args.action==='create'||args.action==='mount')diskState='mounted';if(args.action==='detach')diskState='detached';if(args.action==='enable-auto')storage.autoMount=true;return {state:diskState,message:diskState==='mounted'?'已挂载':'尚未创建',target:diskState==='mounted'?'V:\\AgentData':'',size:1024**3,free:900*1024**2,backingFree:4*1024**3,imageBytes:32*1024**2,autoMount:storage.autoMount};}
        case 'get_snapshot':case 'start_scan':return snapshot;
        case 'get_overview':return {...snapshot,selectedVolume:args.volume,agentSubtotal:snapshot.totalBytes,agentFiles:{codex:2}};
        case 'list_entities':return args.kind==='agents'?{items:snapshot.agents,total:snapshot.agents.length}:{items:[],total:0};
        case 'get_move_candidates':return [{...candidate,available:!jobs.some(j=>!['restored','cancelled','planned'].includes(String(j.phase)))}];
        case 'get_move_jobs':return jobs;
        case 'prepare_move':{const plan={id:'019f0986-2794-7af1-aad1-8c96afb880f1',candidate:candidate.id,agent:'codex',label:candidate.label,source:candidate.source,destination:'R:\\synthetic\\agent-space-job',bytes:4194304,allocatedBytes:4194304,fileCount:2,crossVolume:true,warning:'合成桥接方案，未访问真实文件'};jobs=[{...plan,status:'planned',phase:'planned',message:'方案',backup:'synthetic-backup',retainedBytes:0,freedBytes:0,createdAt:'1793977200'}];return plan;}
        case 'run_move':{const j=jobs.find(j=>j.id===args.id)!;const action=args.action;j.phase=action==='execute'?'active':action==='finalize'?'committed':j.phase==='planned'?'cancelled':'restored';j.status=j.phase;j.retainedBytes=action==='execute'?4194304:0;j.freedBytes=action==='finalize'?4194304:0;j.message='合成测试完成';return j;}
        case 'export_report':return `C:\\synthetic\\exports\\agent-space-test.${args.format}`;
        case 'open_export':case 'reveal_entity':return null;
        default:throw Error('Unexpected command '+command);
      }
    }}});
  });
});

test('native location accepts an indexed identity rather than a path argument',async({page})=>{
  await page.goto('/');
  await page.getByRole('button',{name:'定位Codex',exact:true}).click();
  const calls=await page.evaluate(()=>((window as unknown as {__testCalls:{command:string;args:Record<string,unknown>}[]}).__testCalls));
  expect(calls.find(c=>c.command==='reveal_entity')?.args).toEqual({kind:'agents',id:'codex'});
});

test('custom titlebar sends minimize, maximize, restore and close to the native window',async({page})=>{
  await page.goto('/');
  await page.getByRole('button',{name:'最大化窗口',exact:true}).click();
  await expect(page.getByRole('button',{name:'还原窗口',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'还原窗口',exact:true}).click();
  await expect(page.getByRole('button',{name:'最大化窗口',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'最小化窗口',exact:true}).click();
  await page.getByRole('button',{name:'关闭窗口',exact:true}).click();
  const calls=await page.evaluate(()=>((window as unknown as {__testCalls:{command:string;args:Record<string,unknown>}[]}).__testCalls));
  expect(calls.filter(c=>['plugin:window|toggle_maximize','plugin:window|minimize','plugin:window|close'].includes(c.command)).map(c=>c.command)).toEqual(['plugin:window|toggle_maximize','plugin:window|toggle_maximize','plugin:window|minimize','plugin:window|close']);
});

test('native virtual disk requires saved settings and explicit action confirmation',async({page})=>{
  await page.goto('/');await page.getByRole('button',{name:'设置',exact:true}).click();
  await page.getByRole('combobox',{name:'存储方式'}).selectOption('vhdx');
  await expect(page.getByRole('button',{name:'创建并挂载',exact:true})).toBeDisabled();
  await page.getByRole('button',{name:'保存设置',exact:true}).click();
  await page.getByRole('button',{name:'创建并挂载',exact:true}).click();
  let calls=await page.evaluate(()=>((window as unknown as {__testCalls:{command:string;args:Record<string,unknown>}[]}).__testCalls));
  expect(calls.filter(c=>c.command==='virtual_disk').every(c=>c.args.action==='status')).toBe(true);
  await page.getByRole('dialog').getByRole('button',{name:'确认创建并挂载',exact:true}).click();
  await expect(page.locator('.disk-properties')).toContainText('已挂载');
  await expect(page.getByRole('textbox',{name:'镜像文件'})).toBeDisabled();
  await expect(page.locator('.disk-properties')).toContainText('4.0 GB');
  await page.getByRole('button',{name:'启用自动挂载',exact:true}).click();
  await page.getByRole('dialog').getByRole('button',{name:'确认启用登录时挂载',exact:true}).click();
  await expect(page.getByRole('button',{name:'关闭自动挂载',exact:true})).toBeEnabled();
  calls=await page.evaluate(()=>((window as unknown as {__testCalls:{command:string;args:Record<string,unknown>}[]}).__testCalls));
  expect(calls.filter(c=>c.command==='virtual_disk'&&c.args.action!=='status').map(c=>c.args)).toEqual([{action:'create'},{action:'enable-auto'}]);
});

test('ordinary Windows uses local scan without an enrollment toggle',async({page})=>{
  await page.goto('/');
  await expect(page.getByRole('button',{name:'扫描本机',exact:true})).toBeVisible();
  await page.locator('.sidebar').getByRole('button',{name:'帮助',exact:true}).click();
  await expect(page.getByRole('dialog')).not.toContainText('VMware');
  await expect(page.getByRole('dialog').getByRole('button',{name:/启用|登记|切换/})).toHaveCount(0);
  await page.getByRole('dialog').getByRole('button',{name:'关闭',exact:true}).last().click();
  const calls=await page.evaluate(()=>((window as unknown as {__testCalls:{command:string}[]}).__testCalls));
  expect(calls.some(c=>c.command==='start_scan')).toBe(false);
  await page.getByRole('button',{name:'扫描本机',exact:true}).click();
  await expect(page.getByRole('status')).toContainText('扫描完成');
});

test('real migration preview, separate cleanup confirmation, and restore use recorded IDs',async({page})=>{
  await page.goto('/');
  await expect(page.getByText('42 个对话',{exact:true})).toBeVisible();
  await page.getByRole('button',{name:'迁移记录',exact:true}).click();
  const preview=page.getByRole('button',{name:'预览方案'});
  await expect(preview).toBeDisabled();
  await page.getByRole('textbox',{name:'迁移目标根目录'}).fill('R:\\synthetic');
  await preview.click();
  await expect(page.getByRole('dialog')).toContainText('C:\\synthetic\\sessions');
  await page.getByRole('button',{name:'确认并开始迁移'}).click();
  await expect(page.getByText('已迁移 · 保留副本',{exact:true})).toBeVisible();
  await page.getByRole('button',{name:'验证并清理副本'}).click();
  await expect(page.getByRole('dialog')).toContainText('重新校验目标和原副本');
  await page.getByRole('button',{name:'确认清理'}).click();
  await expect(page.getByText('已清理原副本',{exact:true})).toBeVisible();
  await page.getByRole('button',{name:'恢复原位置',exact:true}).click();
  await page.getByRole('button',{name:'确认恢复'}).click();
  await expect(page.getByText('已恢复',{exact:true})).toBeVisible();
  const operations=await page.evaluate(()=>((window as unknown as {__testCalls:{command:string;args:Record<string,unknown>}[]}).__testCalls).filter(x=>x.command==='run_move').map(x=>({id:x.args.id,action:x.args.action})));
  expect(operations.map(x=>x.action)).toEqual(['execute','finalize','restore']);expect(new Set(operations.map(x=>x.id)).size).toBe(1);
});

test('cancelled preview is discarded and native report exposes its saved path',async({page})=>{
  await page.goto('/');await page.getByRole('button',{name:'迁移记录',exact:true}).click();
  await page.getByRole('textbox',{name:'迁移目标根目录'}).fill('R:\\synthetic');await page.getByRole('button',{name:'预览方案'}).click();await page.getByRole('button',{name:'取消方案',exact:true}).click();
  await expect(page.getByText('已取消',{exact:true})).toBeVisible();
  await page.getByLabel('报告格式').selectOption('csv');await page.getByRole('button',{name:'导出报告'}).click();
  await expect(page.getByRole('dialog')).toContainText('agent-space-test.csv');await page.getByRole('button',{name:'在文件夹中查看'}).click();
  const calls=await page.evaluate(()=>((window as unknown as {__testCalls:{command:string;args:Record<string,unknown>}[]}).__testCalls));expect(calls.find(x=>x.command==='export_report')?.args.format).toBe('csv');expect(calls.some(x=>x.command==='open_export')).toBe(true);
});
