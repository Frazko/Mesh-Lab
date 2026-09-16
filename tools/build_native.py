#!/usr/bin/env python3
"""Build the Rust mobile packages. Run before Flutter build/run; no global config changes."""
import argparse, os, pathlib, shutil, subprocess
ROOT = pathlib.Path(__file__).resolve().parents[1]
CARGO = pathlib.Path.home()/'.cargo/bin/cargo'
RUSTUP = pathlib.Path.home()/'.cargo/bin/rustup'

def run(*args, env=None):
    subprocess.run([str(a) for a in args], cwd=ROOT, env=env, check=True)

def apple():
    targets=['aarch64-apple-ios','aarch64-apple-ios-sim','x86_64-apple-ios']
    run(RUSTUP,'target','add',*targets)
    for target in targets:
        # Xcode 26 defaults C sources to the SDK's deployment version when this
        # is absent. Keep Rust, C, and the Flutter runner on the same iOS floor.
        env=os.environ.copy()
        env.pop('SDKROOT', None)
        env['IPHONEOS_DEPLOYMENT_TARGET']='15.0'
        rustflags='-C link-arg=-miphoneos-version-min=15.0' if target == 'aarch64-apple-ios' else '-C link-arg=-mios-simulator-version-min=15.0'
        env['CARGO_TARGET_'+target.upper().replace('-','_')+'_RUSTFLAGS']=rustflags
        run(CARGO,'build','--locked','--release','-p','mesh-ffi-c','--target',target,env=env)
    headers=ROOT/'target/mesh-headers'; headers.mkdir(exist_ok=True)
    shutil.copy(ROOT/'platforms/mesh_host/ios/Classes/include/mesh_engine.h',headers)
    (headers/'module.modulemap').write_text('module MeshEngine {\n  header "mesh_engine.h"\n  export *\n}\n')
    output=ROOT/'platforms/mesh_host/ios/mesh_host/MeshEngine.xcframework'
    if output.exists(): shutil.rmtree(output)
    simulator=ROOT/'target/libmesh_ffi_c_sim.a'
    run('lipo','-create',ROOT/'target/aarch64-apple-ios-sim/release/libmesh_ffi_c.a',ROOT/'target/x86_64-apple-ios/release/libmesh_ffi_c.a','-output',simulator)
    args=['xcodebuild','-create-xcframework']
    for library in [ROOT/'target/aarch64-apple-ios/release/libmesh_ffi_c.a',simulator]:
        args+=['-library',str(library),'-headers',str(headers)]
    run(*args,'-output',output)

def android():
    sdk=pathlib.Path(os.environ.get('ANDROID_HOME',pathlib.Path.home()/'Library/Android/sdk'))
    ndk=sdk/'ndk/28.2.13676358'
    host='darwin-x86_64' if os.uname().sysname=='Darwin' else 'linux-x86_64'
    bin_path=ndk/'toolchains/llvm/prebuilt'/host/'bin'
    if not bin_path.is_dir(): raise SystemExit(f'Install pinned NDK 28.2.13676358 in {sdk}')
    triples=[('aarch64-linux-android','arm64-v8a'),('x86_64-linux-android','x86_64')]
    run(RUSTUP,'target','add',*[t for t,_ in triples])
    for target,abi in triples:
        env=os.environ.copy()
        compiler=str(bin_path/f'{target}24-clang')
        env['CARGO_TARGET_'+target.upper().replace('-','_')+'_LINKER']=compiler
        # Cargo's linker setting is not used by C build scripts. SQLCipher and
        # its vendored crypto provider must use this exact NDK compiler too.
        env['CC_'+target]=compiler
        env['CC_'+target.replace('-','_')]=compiler
        env['AR_'+target]=str(bin_path/'llvm-ar')
        env['AR_'+target.replace('-','_')]=str(bin_path/'llvm-ar')
        env['RANLIB_'+target]=str(bin_path/'llvm-ranlib')
        env['RANLIB_'+target.replace('-','_')]=str(bin_path/'llvm-ranlib')
        run(CARGO,'build','--locked','--release','-p','mesh-ffi-jni','--target',target,env=env)
        dest=ROOT/f'platforms/mesh_host/android/src/main/jniLibs/{abi}'
        dest.mkdir(parents=True,exist_ok=True)
        shutil.copy(ROOT/f'target/{target}/release/libmesh_ffi_jni.so',dest)

if __name__=='__main__':
    parser=argparse.ArgumentParser(); parser.add_argument('platform',choices=['apple','android','all'])
    args=parser.parse_args()
    if args.platform in ('apple','all'): apple()
    if args.platform in ('android','all'): android()
