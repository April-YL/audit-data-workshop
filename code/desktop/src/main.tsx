import React from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import Workbench from './workbench';
import GridCompatibilityProbe from './components/GridCompatibilityProbe';

const probe=new URLSearchParams(location.search).get('component-probe')==='1';
if (probe) {
  void listen('project-close-requested', () => {
    void invoke('confirm_project_close');
  });
}
createRoot(document.getElementById('root')!).render(<React.StrictMode>{probe?<GridCompatibilityProbe/>:<><Workbench /><a href="?component-probe=1" onClick={event=>{if(!window.confirm('进入组件验证会离开当前项目，请先保存。是否继续？'))event.preventDefault();}} style={{position:'fixed',right:8,bottom:4,zIndex:100,fontSize:11}}>组件验证（虚构资料）</a></>}</React.StrictMode>);
