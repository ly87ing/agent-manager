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
function git(args) {
  const result=spawnSync("git",args,{cwd:work,env:{...env,GIT_CONFIG_NOSYSTEM:"1",GIT_CONFIG_GLOBAL:process.platform==="win32"?"NUL":"/dev/null"},encoding:"utf8",timeout:30_000});
  assert.ifError(result.error);assert.equal(result.status,0,result.stderr);
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
  write(".claude/projects/project/11111111-1111-4111-8111-111111111111.jsonl","PRIVATE_SESSION");
  write(".claude/projects/project/memory/MEMORY.md","PRIVATE_MEMORY");
  write(".claude.json",JSON.stringify({oauthAccount:{id:"PRIVATE_ACCOUNT_ID"}}));
  const configurationBackups=[".claude/backups/.claude.json.backup.1770000000000",".claude/backups/.claude.json.corrupted.1770000000001",".claude.json.backup.1770000000002",".claude.json.backup.1770000000003"];
  for(const name of configurationBackups)write(name,"PRIVATE_CONFIGURATION_BACKUP");
  write("PERSONAL_DOCUMENT.md","PERSONAL_DOCUMENT");
  write("project/.claude/settings.json","PROJECT_SOURCE");
  fs.mkdirSync(path.join(work,"external")); fs.writeFileSync(path.join(work,"external/source"),"EXTERNAL_SOURCE");
  fs.mkdirSync(path.join(root,"skills"));
  fs.symlinkSync(path.join(work,"external"),path.join(root,"skills/external"),process.platform==="win32"?"junction":"dir");
  const repository=path.join(work,"source-repository"),worktree=path.join(root,"worktrees/source"),personal=path.join(root,"skills/personal");
  git(["init","--quiet","--initial-branch=main",repository]);
  fs.writeFileSync(path.join(repository,"SOURCE.txt"),"ORIGINAL_PROJECT_SOURCE");
  git(["-C",repository,"add","SOURCE.txt"]);
  git(["-C",repository,"-c","user.name=YOUR_TEST_USER","-c","user.email=YOUR_TEST_USER@example.invalid","-c","commit.gpgsign=false","-c",`core.hooksPath=${path.join(work,"unused-hooks")}`,"commit","--quiet","-m","fixture"]);
  git(["-C",repository,"worktree","add","--quiet","--detach",worktree]);
  git(["clone","--quiet","--no-hardlinks",repository,personal]);
  write(".claude/worktrees/source/SOURCE.txt","UNCOMMITTED_PROJECT_SOURCE");
  write(".claude/worktrees/source/UNTRACKED.txt","UNTRACKED_PROJECT_SOURCE");
  write(".claude/worktrees/without-vcs/main.py","PROJECT_WITHOUT_VCS");
  write(".claude/skills/personal/SOURCE.txt","UNCOMMITTED_PERSONAL_SOURCE");
  if(process.env.AM_TEST_CLAUDE_BIND_MOUNT==="1") {
    assert(process.platform==="linux" && process.env.GITHUB_ACTIONS==="true","Bind-mount acceptance requires a disposable Linux runner");
    const directory=path.join(root,"skills/mounted");fs.mkdirSync(directory);
    mountRefusal(path.join(work,"external"),directory);fs.rmdirSync(directory);
    const file=path.join(root,".device-keys.json");fs.writeFileSync(file,"");
    mountRefusal(path.join(work,"external/source"),file);fs.unlinkSync(file);
    mountRefusal(path.join(work,"external"),root);
  }
  const before=fs.readFileSync(path.join(root,"settings.json"));
  const preview=run("preview",["--files-only"]);
  assert(preview.ok); assert(preview.data.can_clear); assert(!fs.existsSync(data));
  assert.equal(preview.data.retained.filter(item=>item.reason==="project").length,2);
  for(const name of configurationBackups)assert(preview.data.configuration_paths.includes(fs.realpathSync(path.join(home,name))));
  assert.deepEqual(fs.readFileSync(path.join(root,"settings.json")),before);
  assert.equal(run("clear",["--base",preview.data.base,"--files-only"]).ok,false);
  assert(!fs.existsSync(data));
  write(".claude/.credentials.json","CHANGED_PRIVATE_ACCOUNT");
  assert.equal(run("clear",["--base",preview.data.base,"--confirm","--files-only"]).ok,false);
  assert(!fs.existsSync(data)); assert(fs.existsSync(path.join(root,"projects/project/11111111-1111-4111-8111-111111111111.jsonl")));
  const dependency=run("preview",["--files-only"],{SSL_CERT_FILE:path.join(root,"history.jsonl")});
  assert(!dependency.ok); assert(!fs.existsSync(data));
  const sharedDirectory=run("preview",["--files-only"],{SSL_CERT_DIR:[path.join(work,"external"),root].join(path.delimiter)});
  assert(!sharedDirectory.ok); assert(!fs.existsSync(data));

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
  assert(cleared.ok,JSON.stringify(cleared.errors)); assert.equal(cleared.data.status,"complete"); assert.equal(cleared.data.scope_verified,true); assert.equal(cleared.data.retained_verified,true);
  assert.equal(cleared.data.deleted_credentials,0); assert.equal(cleared.data.deleted_files,11);
  assert.deepEqual(fs.readdirSync(root),["skills","worktrees"]);
  assert(!fs.existsSync(path.join(root,"settings.json")));
  for(const name of configurationBackups)assert(!fs.existsSync(path.join(home,name)));
  assert.equal(fs.readFileSync(path.join(home,"PERSONAL_DOCUMENT.md"),"utf8"),"PERSONAL_DOCUMENT");
  assert.equal(cleared.data.telemetry_policy,"remove");
  assert(!fs.existsSync(path.join(home,".claude.json")));
  assert.equal(fs.readFileSync(path.join(home,"project/.claude/settings.json"),"utf8"),"PROJECT_SOURCE");
  assert.equal(fs.readFileSync(path.join(work,"external/source"),"utf8"),"EXTERNAL_SOURCE");
  for(const [name,bytes] of [["worktrees/source/SOURCE.txt","UNCOMMITTED_PROJECT_SOURCE"],["worktrees/source/UNTRACKED.txt","UNTRACKED_PROJECT_SOURCE"],["worktrees/without-vcs/main.py","PROJECT_WITHOUT_VCS"],["skills/personal/SOURCE.txt","UNCOMMITTED_PERSONAL_SOURCE"]])assert.equal(fs.readFileSync(path.join(root,name),"utf8"),bytes);
  assert.equal(fs.readFileSync(path.join(repository,"SOURCE.txt"),"utf8"),"ORIGINAL_PROJECT_SOURCE");
  write(".claude.json.backup.notes.md","PERSONAL_DOCUMENT");
  write(".claude/backups/.claude.json.backup.1770000000004","PRIVATE_MIXED_BACKUP");
  write(".claude/backups/README.md","PERSONAL_DOCUMENT");
  write(".claude/personal-source/main.rs","UNCLASSIFIED_SOURCE");
  write(".claude/personal-source/README.md","UNCLASSIFIED_DOC");
  write(".claude/history.jsonl","PRIVATE_NEW_HISTORY");
  const unknown=run("preview",["--files-only"]);assert(unknown.ok&&unknown.data.can_clear);assert(unknown.data.retained.some(item=>item.reason==="unclassified"));
  const partial=run("clear",["--base",unknown.data.base,"--confirm","--files-only"]);assert(!partial.ok);assert.equal(partial.data.status,"partial");assert.equal(partial.data.retained_verified,true);assert(!fs.existsSync(path.join(root,"history.jsonl")));
  assert.equal(fs.readFileSync(path.join(root,"personal-source/main.rs"),"utf8"),"UNCLASSIFIED_SOURCE");assert.equal(fs.readFileSync(path.join(root,"personal-source/README.md"),"utf8"),"UNCLASSIFIED_DOC");
  assert.equal(fs.readFileSync(path.join(home,".claude.json.backup.notes.md"),"utf8"),"PERSONAL_DOCUMENT");
  assert(partial.data.retained.some(item=>item.path===fs.realpathSync(path.join(home,".claude.json.backup.notes.md"))&&item.reason==="unclassified"));
  assert.equal(fs.readFileSync(path.join(root,"backups/README.md"),"utf8"),"PERSONAL_DOCUMENT");
  assert.equal(fs.readFileSync(path.join(root,"backups/.claude.json.backup.1770000000004"),"utf8"),"PRIVATE_MIXED_BACKUP");
  const after=run("preview",["--files-only"]); assert.equal(after.data.has_changes,false);
  const noOp=run("clear",["--base",after.data.base,"--confirm","--files-only"]); assert.equal(noOp.data.status,"not_applied");
  console.log(JSON.stringify({platform:process.platform,architecture:process.arch,scope:"synthetic_files_only",previewZeroWrite:true,staleRefused:true,writerBlocked:true,telemetryRemoved:true,recognizedBackupsRemoved:true,externalSourcePreserved:true,worktreesPreserved:true,unclassifiedPreserved:true,status:"passed"}));
} finally {
  if(writer) {const stopped=new Promise(resolve=>writer.once("exit",resolve));writer.kill();await stopped;}
  unmount();
  fs.rmSync(work,{recursive:true,force:true});
}
