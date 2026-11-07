import { create } from 'zustand';
import type { View } from './types';
interface UiState { view:View; query:string; setView:(v:View)=>void; setQuery:(q:string)=>void }
export const useUi = create<UiState>(set=>({view:'overview',query:'',setView:view=>set({view,query:''}),setQuery:query=>set({query})}));
