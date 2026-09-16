#!/usr/bin/env python3
"""Produce a local build inventory and Rust CycloneDX SBOM; never signs or publishes."""
import datetime, hashlib, json, pathlib, subprocess, sys
ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'artifacts'; OUT.mkdir(exist_ok=True)
def run(*args,cwd=ROOT): return subprocess.check_output(args,cwd=cwd,text=True).strip()
def digest(path):
    h=hashlib.sha256()
    with open(path,'rb') as f:
        for chunk in iter(lambda:f.read(1024*1024),b''): h.update(chunk)
    return h.hexdigest()
files=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z'],cwd=ROOT).decode().split('\0')
source={p:digest(ROOT/p) for p in sorted(set(files)) if p and (ROOT/p).is_file()}
tree=hashlib.sha256(json.dumps(source,sort_keys=True).encode()).hexdigest()
commit_result=subprocess.run(['git','rev-parse','--verify','HEAD'],cwd=ROOT,text=True,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
commit=commit_result.stdout.strip() if commit_result.returncode==0 else None
manifest={'schemaVersion':1,'createdAt':datetime.datetime.now(datetime.timezone.utc).isoformat(),
          'sourceCommit':commit,'sourceInventoryAtReportTimeSha256':tree,'sourceFiles':source,
          'buildKind':'local-development','abi':1,'api':1,
          'rust':run(str(pathlib.Path.home()/'.cargo/bin/rustc'),'--version'),
          'flutter':json.loads(run('flutter','--version','--machine')),
          'limitations':['Native bridge validated on physical iOS/Android; no radio validation','Android APK is debug-signed; no store distribution','Rust SBOM excludes Flutter SDK and Gradle/Maven dependencies; those are inventoried separately'],
          'artifacts':[]}
for pattern in ['app/build/app/outputs/flutter-apk/*.apk','app/build/mesh_host/outputs/aar/*.aar','platforms/mesh_host/ios/mesh_host/MeshEngine.xcframework/**/*.a']:
    for p in sorted(ROOT.glob(pattern)):
        manifest['artifacts'].append({'path':str(p.relative_to(ROOT)),'sha256':digest(p),'bytes':p.stat().st_size})
(OUT/'build-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
metadata=json.loads(run(str(pathlib.Path.home()/'.cargo/bin/cargo'),'metadata','--locked','--format-version','1'))
components=[]
for p in metadata['packages']:
    item={'type':'library','bom-ref':p['id'],'name':p['name'],'version':p['version']}
    if p['source']: item['purl']=f"pkg:cargo/{p['name']}@{p['version']}"
    if p['license']: item['licenses']=[{'expression':p['license']}]
    components.append(item)
sbom={'bomFormat':'CycloneDX','specVersion':'1.5','version':1,
      'components':components,'dependencies':[{'ref':n['id'],'dependsOn':n['dependencies']} for n in metadata['resolve']['nodes']]}
(OUT/'rust-sbom.cdx.json').write_text(json.dumps(sbom,indent=2)+'\n')
(OUT/'flutter-dependencies.json').write_text(run('flutter','pub','deps','--json',cwd=ROOT/'app')+'\n')
print(f'Wrote build manifest, Rust SBOM and Flutter dependency inventory to {OUT}')
