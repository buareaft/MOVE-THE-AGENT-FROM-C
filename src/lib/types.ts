export type View = 'overview' | 'projects' | 'sessions' | 'migrations' | 'settings';
export interface StorageSettings {backend:'directory'|'vhdx';directory:string;imagePath:string;capacityGb:number;driveLetter:string;autoMount:boolean;diskId:string}
export interface VirtualDiskStatus {state:string;message:string;target?:string;size?:number;free?:number;backingFree?:number;imageBytes?:number;autoMount?:boolean}
export type Kind = 'agents' | 'projects' | 'sessions';
export interface Entity { id: string; name: string; agent: string; path: string; bytes: number; allocated: number; category: string; sessions: number; updated: string; note: string }
export interface Job { id: string; agent: string; source: string; destination: string; backup: string; status: string; bytes: number; createdAt: string; message: string; hashes: [string, string][] }
export interface Snapshot { mode: string; root: string; scannedAt: string; totalBytes: number; allocatedBytes: number; fileCount: number; exclusiveBytes: number; sharedBytes: number; projectBytes: number; unknownBytes: number; backupBytes: number; agents: Entity[]; projects: Entity[]; sessions: Entity[]; projectCount?:number;sessionCount?:number;volumes?:{volume:string;bytes:number;allocatedBytes:number;files:number}[];jobs: Job[]; warnings: string[] }
export interface Plan { agent: string; source: string; destination: string; bytes: number; fileCount: number; steps: string[]; warning: string }
export interface Page { items: Entity[]; total: number }
export interface Overview extends Snapshot {agentFiles:Record<string,number>;agentSubtotal:number;selectedVolume:string}
export interface Progress { phase: string; files: number; done: boolean }
export interface Breakdown { label: string; bytes: number; files: number }
export interface RuntimeInfo { windowsSupported:boolean;dataMode:string;message:string }
export interface MoveCandidate {id:string;agent:string;label:string;source:string;available:boolean;reason:string}
export interface MovePlan {id:string;candidate:string;agent:string;label:string;source:string;destination:string;bytes:number;allocatedBytes:number;fileCount:number;crossVolume:boolean;warning:string}
export interface MoveJob extends MovePlan {status:string;phase:string;message:string;backup:string;retainedBytes:number;freedBytes:number;createdAt:string}
export interface MoveProgress {phase:string;files:number;bytes:number;totalBytes:number}
export const formatBytes = (n: number) => { if (!n) return '0 B'; const units = ['B', 'KB', 'MB', 'GB', 'TB']; const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), 4); return `${(n / 1024 ** i).toFixed(i > 0 ? 1 : 0)} ${units[i]}`; };
export const agentName = (id: string) => ({ codex: 'Codex', claude: 'Claude Code', deepseek:'DeepSeek Harness', workspace: '工作项目', chatgpt:'ChatGPT/Codex 桌面数据', 'codex-desktop':'Codex 桌面运行数据' }[id] ?? id);
