// Read-only UI inspection against a separately launched Chrome (port 9223).
// Set CSC_GAME_ID to inspect an existing game. No game mutations are performed.
import {mkdir,writeFile} from 'node:fs/promises';
const pages=await (await fetch('http://127.0.0.1:9223/json')).json();
const ws=new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);
await new Promise(r=>ws.addEventListener('open',r,{once:true}));let seq=0;const pending=new Map();
ws.addEventListener('message',event=>{const msg=JSON.parse(event.data);if(msg.id&&pending.has(msg.id)){const [resolve,reject]=pending.get(msg.id);pending.delete(msg.id);msg.error?reject(msg.error):resolve(msg.result)}});
const cdp=(method,params={})=>new Promise((resolve,reject)=>{const id=++seq;pending.set(id,[resolve,reject]);ws.send(JSON.stringify({id,method,params}))});
const wait=ms=>new Promise(r=>setTimeout(r,ms));const output='../docs/screenshots/career-v1';await mkdir(output,{recursive:true});await cdp('Page.enable');await cdp('Runtime.enable');
await cdp('Page.navigate',{url:'http://127.0.0.1:5173/'});await wait(800);
if(process.env.CSC_GAME_ID){await cdp('Runtime.evaluate',{expression:`localStorage.setItem('csc-game',${JSON.stringify(process.env.CSC_GAME_ID)});location.reload()`});await wait(1500)}
const report=[];
for(const width of [1440,390]){await cdp('Emulation.setDeviceMetricsOverride',{width,height:width===390?844:900,deviceScaleFactor:1,mobile:false});
for(const page of process.env.CSC_GAME_ID?['career','schedule','team','world','archive','settings']:['career']){
await cdp('Runtime.evaluate',{expression:`location.hash=${JSON.stringify(page)}`});await wait(1000);
const dom=await cdp('Runtime.evaluate',{expression:`JSON.stringify({title:document.querySelector('h1')?.textContent,body:document.body.innerText.slice(0,6000),width:innerWidth,scrollWidth:document.documentElement.scrollWidth,buttons:[...document.querySelectorAll('button')].map(b=>({text:b.textContent,disabled:b.disabled})),errors:[...document.querySelectorAll('.alert')].map(n=>n.textContent)})`,returnByValue:true});
const state=JSON.parse(dom.result.value);report.push({page,width,...state});
const screenshot=await cdp('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(`${output}/${page}-${width}-${process.env.CSC_GAME_ID?'game':'new'}.png`,Buffer.from(screenshot.data,'base64'));
}}
await writeFile(`${output}/dom-${process.env.CSC_GAME_ID?'game':'new'}.json`,JSON.stringify(report,null,2));console.log(JSON.stringify(report.map(({page,width,scrollWidth,title,errors})=>({page,width,scrollWidth,title,errors})),null,2));ws.close();
