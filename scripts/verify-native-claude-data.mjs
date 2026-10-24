// Disposable filesystem acceptance. This never queries the operating system credential store.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";

const binary = path.resolve(process.argv[2] ?? `engine-rs/target/debug/agent-manager-engine${process.platform === "win32" ? ".exe" : ""}`);
const work = fs.mkdtempSync(path.join(os.tmpdir(), "am-claude-native-"));
const home = path.join(work, "home"), data = path.join(work, "manager"), root = path.join(home, ".claude");
const env = { ...process.env, HOME: home, USERPROFILE: home, APPDATA: path.join(home,"appdata"), XDG_DATA_HOME: path.join(home,"data"), AGENT_MANAGER_DATA_DIR: data };
const settings = { feedbackDrafts: "off", env: { DISABLE_TELEMETRY: "0", OTEL_EXPORTER_OTLP_HEADERS: "PRIVATE_TELEMETRY", ANTHROPIC_API_KEY: "PRIVATE_ACCOUNT" }, model: "old" };
function write(name, value) { const file=path.join(home,name); fs.mkdirSync(path.dirname(file),{recursive:true}); fs.writeFileSync(file,value); }
function run(verb, args = [], override = {}) {
  const result=spawnSync(binary,[`claude-data:${verb}`,"--json",...args],{cwd:work,env:{...env,...override},encoding:"utf8",timeout:60_000,maxBuffer:1024*1024});
  assert.ifError(result.error);
  const envelope=JSON.parse(result.stdout);
  assert.equal(result.status,envelope.ok?0:1);
  assert(!result.stdout.includes("PRIVATE_"));
  return envelope;
}
let writer;
let mounted;
function unmount() {
  if(!mounted) return;
  const result=spawnSync("sudo",["-n","umount",mounted],{encoding:"utf8"});
  assert.equal(result.status,0,"Fixture unmount failed; mounted data must be retained");
  mounted=undefined;
}
function mountRefusal(source,target) {
  const result=spawnSync("sudo",["-n","mount","--bind",source,target],{encoding:"utf8"});
  assert.equal(result.status,0,"Disposable bind-mount prerequisite failed");mounted=target;
  const response=run("preview",["--files-only"]); assert(!response.ok); assert(!fs.existsSync(data));
  assert.equal(fs.readFileSync(path.join(work,"external/source"),"utf8"),"EXTERNAL_SOURCE");
  unmount();
}
try {
  write(".claude/settings.json",JSON.stringify(settings));
  write(".claude/.credentials.json","PRIVATE_ACCOUNT");
  write(".claude/history.jsonl","PRIVATE_HISTORY");
  write(".claude/projects/project/session.jsonl","PRIVATE_SESSION");
  write(".claude/projects/project/memory/MEMORY.md","PRIVATE_MEMORY");
  write(".claude.json",JSON.stringify({oauthAccount:{id:"PRIVATE_ACCOUNT_ID"}}));
  write("project/.claude/settings.json","PROJECT_SOURCE");
  fs.mkdirSync(path.join(work,"external")); fs.writeFileSync(path.join(work,"external/source"),"EXTERNAL_SOURCE");
  fs.mkdirSync(path.join(root,"skills"));
  fs.symlinkSync(path.join(work,"external"),path.join(root,"skills/external"),process.platform==="win32"?"junction":"dir");
  if(process.env.AM_TEST_CLAUDE_BIND_MOUNT==="1") {
    assert(process.platform==="linux" && process.env.GITHUB_ACTIONS==="true","Bind-mount acceptance requires a disposable Linux runner");
    const directory=path.join(root,"skills/mounted");fs.mkdirSync(directory);
    mountRefusal(path.join(work,"external"),directory);fs.rmdirSync(directory);
    const file=path.join(root,"mounted-file");fs.writeFileSync(file,"");
    mountRefusal(path.join(work,"external/source"),file);fs.unlinkSync(file);
    mountRefusal(path.join(work,"external"),root);
  }
  const before=fs.readFileSync(path.join(root,"settings.json"));
  const preview=run("preview",["--files-only"]);
  assert(preview.ok); assert(preview.data.can_clear); assert(!fs.existsSync(data));
  assert.deepEqual(fs.readFileSync(path.join(root,"settings.json")),before);
  assert.equal(run("clear",["--base",preview.data.base,"--files-only"]).ok,false);
  assert(!fs.existsSync(data));
  write(".claude/.credentials.json","CHANGED_PRIVATE_ACCOUNT");
  assert.equal(run("clear",["--base",preview.data.base,"--confirm","--files-only"]).ok,false);
  assert(!fs.existsSync(data)); assert(fs.existsSync(path.join(root,"projects/project/session.jsonl")));
  const dependency=run("preview",["--files-only"],{OTEL_EXPORTER_OTLP_CERTIFICATE:path.join(root,"certificate.pem")});
  assert(!dependency.ok); assert(!fs.existsSync(data));

  // A real process with a synthetic Claude executable name must block the real process probe.
  const executable=path.join(work,process.platform==="win32"?"claude.exe":"claude");
  fs.copyFileSync(process.execPath,executable); fs.chmodSync(executable,0o700);
  writer=spawn(executable,["-e","process.stdout.write('READY'); setInterval(()=>{},1000)"],{cwd:work,env,stdio:["ignore","pipe","ignore"]});
  await Promise.race([new Promise((resolve,reject)=>{writer.stdout.once("data",resolve);writer.once("error",reject);writer.once("exit",()=>reject(Error("Synthetic writer exited early")));}),new Promise((_,reject)=>setTimeout(()=>reject(Error("Writer start timed out")),10000).unref())]);
  if(process.platform==="linux") {
    const argumentsBytes=fs.readFileSync(`/proc/${writer.pid}/cmdline`);
    assert.equal(argumentsBytes.subarray(0,argumentsBytes.indexOf(0)).toString(),executable,"Synthetic writer executable argument changed");
  }
  const blocked=run("preview",["--files-only"]); assert(blocked.ok); assert.equal(blocked.data.can_clear,false); assert(blocked.data.blockers.length>0);
  const stopped=new Promise(resolve=>writer.once("exit",resolve)); writer.kill(); await stopped; writer=undefined;
  const fresh=run("preview",["--files-only"]); assert(fresh.ok && fresh.data.can_clear);
  const cleared=run("clear",["--base",fresh.data.base,"--confirm","--files-only"]);
  assert(cleared.ok,JSON.stringify(cleared.errors)); assert.equal(cleared.data.status,"complete"); assert.equal(cleared.data.telemetry_verified,true);
  assert.equal(cleared.data.deleted_credentials,0); assert.equal(cleared.data.deleted_files,6);
  assert.deepEqual(fs.readdirSync(root),["settings.json"]);
  const retained=JSON.parse(fs.readFileSync(path.join(root,"settings.json"),"utf8"));
  assert.deepEqual(retained,{feedbackDrafts:"off",env:{DISABLE_TELEMETRY:"0",OTEL_EXPORTER_OTLP_HEADERS:"PRIVATE_TELEMETRY"}});
  assert(!fs.existsSync(path.join(home,".claude.json")));
  assert.equal(fs.readFileSync(path.join(home,"project/.claude/settings.json"),"utf8"),"PROJECT_SOURCE");
  assert.equal(fs.readFileSync(path.join(work,"external/source"),"utf8"),"EXTERNAL_SOURCE");
  const after=run("preview",["--files-only"]); assert.equal(after.data.has_changes,false);
  const noOp=run("clear",["--base",after.data.base,"--confirm","--files-only"]); assert.equal(noOp.data.status,"not_applied");
  console.log(JSON.stringify({platform:process.platform,architecture:process.arch,scope:"synthetic_files_only",previewZeroWrite:true,staleRefused:true,writerBlocked:true,telemetryPreserved:true,externalSourcePreserved:true,status:"passed"}));
} finally {
  if(writer) {const stopped=new Promise(resolve=>writer.once("exit",resolve));writer.kill();await stopped;}
  unmount();
  fs.rmSync(work,{recursive:true,force:true});
}
