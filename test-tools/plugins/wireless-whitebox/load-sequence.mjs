// A single, explicitly bounded CC stimulus. Completion of a command is not
// readback of the physical load input state.
export async function withConstantCurrentLoad({current_a,maximum_a,command,markUncertain,stopped=()=>false,isUncertain=()=>false}, action) {
  if(!Number.isFinite(current_a)||!Number.isFinite(maximum_a)||current_a<0||maximum_a<=0||current_a>maximum_a)throw new Error('LOAD.CURRENT_LIMIT_INVALID');
  if(typeof command!=='function'||typeof markUncertain!=='function'||typeof action!=='function')throw new Error('LOAD.SEQUENCE_INVALID');
  let inputMayBeOn=false,unknown=false,initialOffConfirmed=false;
  const send=async cmd=>{
    try{return await command(cmd);}
    catch(error){
      if(error.quiescent!==true) {
        unknown=true;
        try{await markUncertain('LOAD_STATE_UNKNOWN');}catch(e){error.recovery_error=e.message;}
      }
      throw error;
    }
  };
  const check=()=>{if(stopped())throw new Error('RUN.ABORTED');};
  let result,originalError;
  try {
    check();await send({LoadInput:false});initialOffConfirmed=true;
    check();await send({LoadMode:'CC'});
    check();await send({LoadLevel:{mode:'CC',value:current_a}});
    check();inputMayBeOn=true;await send({LoadInput:true});
    check();result=await action();
  } catch(error) {
    originalError=error;
    if(isUncertain())unknown=true;
    if(!initialOffConfirmed&&!unknown && error.message!=='RUN.ABORTED') {
      unknown=true;try{await markUncertain('LOAD_INITIAL_STATE_UNKNOWN');}catch(e){error.recovery_error=e.message;}
    }
  }
  if(inputMayBeOn && !unknown && !isUncertain()) {
    try{await send({LoadInput:false});}
    catch(error){
      if(!unknown){unknown=true;try{await markUncertain('LOAD_SHUTDOWN_FAILED');}catch(e){error.recovery_error=e.message;}}
      if(originalError)originalError.cleanup_error=error.message;else originalError=error;
    }
  }
  if(originalError)throw originalError;
  return {value:result,load:{mode:'CC',requested_current_a:current_a,maximum_current_a:maximum_a,input_off_command_confirmed:true,physical_off_verified:false}};
}
