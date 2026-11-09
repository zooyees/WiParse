import {createContext,useContext,useState,type ReactNode} from 'react';

const Activity=createContext(true);
export const useWorkspaceActive=()=>useContext(Activity);

/** Lazy first visit, retained work state, with explicit visibility for UI readers. */
export function WorkspaceSlot({active,children}:{active:boolean;children:ReactNode}){
  const parent=useWorkspaceActive(),[visited,setVisited]=useState(active);
  if(active&&!visited)setVisited(true);
  if(!active&&!visited)return null;
  return <Activity.Provider value={parent&&active}><div className="workspace-slot" hidden={!active} style={{display:active?'contents':'none'}}>{children}</div></Activity.Provider>;
}
