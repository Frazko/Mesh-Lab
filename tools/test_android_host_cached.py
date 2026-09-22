#!/usr/bin/env python3
"""Compile Kotlin + run host JNI tests from pinned cached jars, without a Gradle download.
Does NOT run Android Keystore, package an APK/AAR, or replace the Gradle CI lane.
"""
import os
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET
ROOT=Path(__file__).resolve().parents[1]
CACHE=Path.home()/'.gradle/caches/modules-2/files-2.1'
def jar(group,name,version):
    files=list((CACHE/group/name/version).glob('*/*.jar'))
    files=[p for p in files if not p.name.endswith(('-sources.jar','-javadoc.jar'))]
    if len(files)!=1:raise SystemExit(f'Cache missing/ambiguous: {group}:{name}:{version}')
    return files[0]
version=os.environ.get('MESH_KOTLIN_VERSION','2.4.0')
compiler=jar('org.jetbrains.kotlin','kotlin-compiler-embeddable',version)
pom=next(compiler.parent.parent.glob('*/*.pom'))
ns={'m':'http://maven.apache.org/POM/4.0.0'}
compiler_cp=[compiler]
for dep in ET.parse(pom).findall('m:dependencies/m:dependency',ns):
    compiler_cp.append(jar(*(dep.find(f'm:{name}',ns).text for name in ['groupId','artifactId','version'])))
annotations=jar('org.jetbrains','annotations','13.0');compiler_cp.append(annotations)
props=dict(line.split('=',1)for line in (ROOT/'app/android/local.properties').read_text().splitlines() if '=' in line)
flutter=Path(props['flutter.sdk'])/'bin/cache/artifacts/engine/android-arm64/flutter.jar'
android=Path(props['sdk.dir'])/'platforms/android-36/android.jar'
classpath=[android,flutter,annotations,jar('org.jetbrains.kotlin','kotlin-stdlib',version),jar('org.jetbrains.kotlinx','kotlinx-coroutines-core-jvm','1.10.2'),jar('org.jetbrains.kotlinx','kotlinx-coroutines-android','1.10.2'),jar('junit','junit','4.13.2'),jar('org.hamcrest','hamcrest-core','1.3')]
java=Path(os.environ.get('JAVA_HOME','/Applications/Android Studio.app/Contents/jbr/Contents/Home'))/'bin/java'
out=ROOT/'target/android-host-tests';out.mkdir(exist_ok=True)
sources=sorted((ROOT/'platforms/mesh_host/android/src').rglob('*.kt'))
cp=lambda paths:os.pathsep.join(map(str,paths))
subprocess.run([str(java),'-cp',cp(compiler_cp),'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler','-no-stdlib','-no-reflect','-jvm-target','17','-classpath',cp(classpath),'-d',str(out),*map(str,sources)],check=True,cwd=ROOT)
agent=os.environ.get('MESH_JACOCO_AGENT')
coverage=[f'-javaagent:{agent}=destfile={out / "jacoco.exec"}'] if agent else []
subprocess.run([str(java),*coverage,f'-Djava.library.path={ROOT / "target/debug"}','-cp',cp([out,*classpath]),'org.junit.runner.JUnitCore','com.frazko.mesh_host.NativeContractTest','com.frazko.mesh_host.EnrollmentRecordCodecTest','com.frazko.mesh_host.BleWriteQueueTest'],check=True,cwd=ROOT)
print(f'PASS: Kotlin {version} plugin compilation and host JNI tests; device Keystore untested')
