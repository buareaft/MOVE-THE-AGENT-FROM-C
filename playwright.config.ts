import { defineConfig } from '@playwright/test';
import path from 'node:path';
process.env.PLAYWRIGHT_BROWSERS_PATH ??= path.resolve('.tools/playwright');
export default defineConfig({testDir:'./tests/ui',fullyParallel:true,workers:2,use:{baseURL:'http://127.0.0.1:1420',viewport:{width:1440,height:1000},trace:'retain-on-failure'},webServer:{command:'npm run dev',url:'http://127.0.0.1:1420',reuseExistingServer:!process.env.CI},reporter:'list'});
