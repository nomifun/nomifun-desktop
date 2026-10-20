// A stdio-only fixture. No desktop libraries, network, user files or providers.
import {createInterface} from 'node:readline';
import {appendFileSync,existsSync} from 'node:fs';
import {join} from 'node:path';
const generation=process.argv[process.argv.indexOf('--generation')+1];
let binding=0;
const respond=(request,result,extra={})=>process.stdout.write(JSON.stringify({protocol_version:1,request_id:request.request_id,generation,ok:true,completion:'completed',result,...extra})+'\n');
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
for await(const line of createInterface({input:process.stdin})){
 const request=JSON.parse(line);
 if(request.operation==='initialize'){respond(request,{ready:true,pid:process.pid,metadata:{driver_version:'fake-1'}});continue;}
 if(request.operation==='bind_session'){respond(request,{session_handle:`binding-${++binding}`});continue;}
 if(request.operation==='close_session'){respond(request,{closed:true});continue;}
 if(request.operation==='shutdown'){respond(request,{shutdown:true});break;}
 if(request.operation==='call'){
  const args=request.arguments??{};
  appendFileSync(join(process.env.CUA_DRIVER_RS_HOME,'calls.jsonl'),JSON.stringify({label:args.label,generation,session:request.session_handle})+'\n');
  if(args.fake_mode==='crash')process.exit(13);
  if(args.fake_mode==='halfpacket'){process.stdout.write('{"protocol_version":1');await new Promise(()=>setInterval(()=>{},1000));}
  if(args.fake_mode==='hang')await new Promise(()=>setInterval(()=>{},1000));
  if(args.wait_for_file)while(!existsSync(args.wait_for_file))await sleep(5);
  if(args.delay_ms)await sleep(args.delay_ms);
  respond(request,{content:[{type:'text',text:'fixture completed'}],structuredContent:{effect:'confirmed',label:args.label},isError:false});
 }
}
