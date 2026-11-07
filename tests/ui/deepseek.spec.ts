import {test,expect} from '@playwright/test';

test('DeepSeek sessions expose storage detail and the correct migration adapter',async({page})=>{
  // Synthetic native replies only. Real directory migration is tested separately.
  await page.addInitScript(()=>{
    const calls:{command:string;args:Record<string,unknown>}[]=[];
    const entity={id:'deepseek-fixture',name:'DeepSeek Harness 会话 短会话',agent:'deepseek',path:'C:\\synthetic\\.dsh\\sessions\\project\\id\\session.v4.jsonl.zstd',bytes:4096,allocated:8192,category:'exclusive',sessions:0,updated:'',note:'包含历史代文件；不分摊共享缓存。'};
    const agent={...entity,id:'deepseek',name:'DeepSeek Harness',path:'C:\\synthetic\\.dsh',category:'shared',sessions:1};
    const snapshot={mode:'native-local',root:'合成 DeepSeek 测试',scannedAt:'1794000000',totalBytes:4096,allocatedBytes:8192,fileCount:2,exclusiveBytes:4096,sharedBytes:0,projectBytes:0,unknownBytes:0,backupBytes:0,agents:[agent],projects:[],sessions:[entity],projectCount:0,sessionCount:1,jobs:[],warnings:[]};
    Object.assign(window,{isTauri:true,__dshCalls:calls,__TAURI_INTERNALS__:{metadata:{currentWindow:{label:'main'}},transformCallback:()=>1,unregisterCallback:()=>{},invoke:async(command:string,args:Record<string,unknown>={})=>{
      calls.push({command,args});
      switch(command){
        case 'plugin:window|is_maximized':return false;
        case 'plugin:event|listen':return 1;
        case 'plugin:event|unlisten':return;
        case 'get_runtime_info':return {windowsSupported:true,dataMode:'local',message:'合成测试'};
        case 'get_snapshot':return snapshot;
        case 'get_overview':return {...snapshot,selectedVolume:args.volume,agentSubtotal:4096,agentFiles:{deepseek:2}};
        case 'list_entities':return {items:args.kind==='sessions'?[entity]:args.kind==='agents'?[agent]:[],total:args.kind==='projects'?0:1};
        case 'get_space_breakdown':return [{label:'会话记录',bytes:4096,files:2}];
        case 'get_move_candidates':return [{id:'deepseek-sessions',agent:'deepseek',label:'DeepSeek Harness 会话目录',source:'C:\\synthetic\\.dsh\\sessions',available:true,reason:''}];
        case 'get_move_jobs':return [];
        case 'configured_move_target':return 'V:\\AgentData';
        case 'prepare_move':return {id:'deepseek-plan',candidate:args.candidate,agent:'deepseek',label:'DeepSeek Harness 会话目录',source:'C:\\synthetic\\.dsh\\sessions',destination:'V:\\AgentData\\synthetic',bytes:4096,allocatedBytes:8192,fileCount:2,crossVolume:true,warning:'合成方案'};
        default:throw Error('Unexpected command '+command);
      }
    }}});
  });
  await page.goto('/');await expect(page.getByRole('button',{name:'DeepSeek Harness',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'对话分析',exact:true}).click();
  await page.getByRole('textbox',{name:'搜索对话'}).fill('DeepSeek');
  await expect(page.locator('tbody tr')).toHaveCount(1);
  await page.getByRole('button',{name:'DeepSeek Harness 会话 短会话',exact:true}).click();
  await expect(page.getByRole('dialog')).toContainText('会话记录');
  await page.getByRole('button',{name:'关闭详情',exact:true}).click();
  await page.getByRole('button',{name:'迁移记录',exact:true}).click();
  await expect(page.getByRole('textbox',{name:'迁移目标根目录'})).toHaveValue('V:\\AgentData');
  await page.getByRole('button',{name:'预览方案',exact:true}).click();
  await expect(page.getByRole('dialog')).toContainText('C:\\synthetic\\.dsh\\sessions');
  const calls=await page.evaluate(()=>(window as unknown as {__dshCalls:{command:string;args:Record<string,unknown>}[]}).__dshCalls);
  expect(calls.find(c=>c.command==='prepare_move')?.args).toEqual({candidate:'deepseek-sessions',target:'V:\\AgentData'});
});
