import { test, expect } from '@playwright/test';

test('overview is explicitly simulated and navigation/search/detail work',async({page})=>{
  const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto('/');
  await expect(page.getByText('浏览器演示模式')).toBeVisible();
  await expect(page.getByRole('heading',{name:'空间总览'})).toBeVisible();
  await page.getByRole('button',{name:'工作项目',exact:true}).click();
  await page.getByRole('textbox',{name:'搜索项目'}).fill('Atlas');
  await expect(page.locator('tbody tr')).toHaveCount(1);
  await page.getByRole('button',{name:'Atlas 工作台',exact:true}).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByRole('dialog').getByText(/^依赖文件/)).toBeVisible();
  await page.getByRole('button',{name:'关闭详情'}).click();
  await page.getByRole('textbox',{name:'搜索项目'}).fill('不存在');
  await expect(page.getByText('没有找到匹配的项目')).toBeVisible();
  await page.getByRole('button',{name:'对话分析',exact:true}).click();
  await expect(page.locator('tbody tr')).toHaveCount(5);
  await page.getByRole('textbox',{name:'搜索对话'}).fill('Claude');
  await expect(page.locator('tbody tr')).toHaveCount(2);
  expect(errors).toEqual([]);
});

test('migration preview, simulation, persistence, and restore',async({page})=>{
  await page.goto('/');
  await page.getByRole('button',{name:'迁移记录',exact:true}).click();
  await expect(page.getByText('暂无迁移记录')).toBeVisible();
  await page.getByRole('button',{name:'预览方案'}).first().click();
  const dialog=page.getByRole('dialog');
  await expect(dialog.getByText('0 B',{exact:true})).toBeVisible();
  await dialog.getByRole('button',{name:'开始模拟演练'}).click();
  await expect(page.getByText('演练完成',{exact:true})).toBeVisible();
  await page.reload();
  await page.getByRole('button',{name:/迁移记录/}).click();
  await page.getByRole('button',{name:'恢复原目录'}).click();
  await expect(page.getByText('已恢复',{exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'预览方案'}).first()).toBeEnabled();
});

test('scan can be cancelled and retried',async({page})=>{
  await page.goto('/');
  await page.getByRole('button',{name:'扫描测试区'}).click();
  await page.getByRole('button',{name:'取消扫描'}).click();
  await expect(page.getByRole('alert')).toContainText('扫描已取消');
  await page.getByRole('button',{name:'扫描测试区'}).click();
  await expect(page.getByRole('button',{name:'扫描测试区'})).toBeEnabled();
  await expect(page.getByRole('status')).toContainText('模拟扫描完成');
});

test('desktop-size and compact layouts stay within the viewport',async({page})=>{
  await page.goto('/');
  await expect(page.getByRole('heading',{name:'空间总览'})).toBeVisible();
  await page.screenshot({path:'artifacts/overview.png',fullPage:true});
  await page.setViewportSize({width:980,height:760});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.locator('.sidebar').getByRole('button',{name:'帮助',exact:true}).click();
  await expect(page.getByRole('dialog')).toContainText('浏览器使用模拟数据');
});

test('disk filtering updates totals, agent subtotal and rows together',async({page,context})=>{
  await context.grantPermissions(['clipboard-read','clipboard-write']);
  await page.goto('/');
  await expect(page.getByRole('img',{name:/共享数据/})).toBeVisible();
  await expect(page.getByText('YOUR SPACE, UNDERSTOOD',{exact:true})).toHaveCount(0);
  await expect(page.getByRole('columnheader',{name:'文件数'})).toBeVisible();
  await page.getByRole('button',{name:'复制Codex路径',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>navigator.clipboard.readText())).toBe('source/agents/codex/cache');
  await expect(page.getByRole('button',{name:'定位Codex',exact:true})).toBeDisabled();
  await expect(page.getByRole('button',{name:'下一页',exact:true})).toHaveCount(0);
  await expect(page.getByRole('columnheader',{name:'Agent',exact:true})).toHaveCount(0);
  await expect(page.locator('.agent-subtotal')).toContainText('1.1 GB');
  await page.getByRole('button',{name:/D: 458/}).click();
  await expect(page.locator('tbody tr')).toHaveCount(1);
  await expect(page.locator('tbody')).toContainText('Claude Code');
  await expect(page.locator('.storage-total')).toHaveText('458.0 MB');
  await expect(page.locator('.agent-subtotal')).toContainText('458.0 MB');
  await expect(page.locator('.summary-counts')).toContainText('2,342 个文件');
  await page.getByRole('button',{name:'全部磁盘',exact:true}).click();
  await expect(page.locator('tbody tr')).toHaveCount(2);
  await expect(page.locator('.storage-total')).toHaveText('2.8 GB');
});
