import {test,expect} from '@playwright/test';

test('settings retain the selected backend and never simulate native disk operations',async({page})=>{
  await page.goto('/');
  await page.getByRole('button',{name:'设置',exact:true}).click();
  await expect(page.getByRole('heading',{name:'设置',exact:true})).toBeVisible();
  await expect(page.getByRole('button',{name:'导出报告'})).toHaveCount(0);
  await page.getByRole('combobox',{name:'存储方式'}).selectOption('vhdx');
  await expect(page.getByRole('textbox',{name:'镜像文件'})).toHaveValue('D:\\MoveYourAgent\\AgentData.vhdx');
  await page.getByRole('textbox',{name:'镜像文件'}).fill('C:\\DiskTest\\data.vhdx');
  await expect(page.getByText('镜像位于 C 盘，迁入此镜像不会释放 C 盘的实际空间。')).toBeVisible();
  await page.getByRole('button',{name:'保存设置',exact:true}).click();
  await expect(page.getByText('已保存演示设置，本次页面有效')).toBeVisible();
  for(const name of ['创建并挂载','挂载','卸载','启用自动挂载'])await expect(page.getByRole('button',{name,exact:true})).toBeDisabled();
  await page.getByRole('button',{name:'空间总览',exact:true}).click();
  await page.getByRole('button',{name:'设置',exact:true}).click();
  await expect(page.getByRole('combobox',{name:'存储方式'})).toHaveValue('vhdx');
  await expect(page.getByRole('textbox',{name:'镜像文件'})).toHaveValue('C:\\DiskTest\\data.vhdx');
  await page.setViewportSize({width:980,height:760});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.screenshot({path:'artifacts/settings-vhdx-demo.png',fullPage:true});
});
