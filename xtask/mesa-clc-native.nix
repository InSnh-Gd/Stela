# Mesa's build-machine programs, for a build host that has Nix.
#
# Mesa compiles the Intel drivers' OpenCL shaders with two programs of its own:
# `mesa_clc` (Blorp's shaders, and the kernels generated draws run) and
# `vtn_bindgen2` (the SPIR-V those produce, turned into the NIR the drivers are
# built from). Meson asks for both as `native: true` programs, so they run on
# the build machine, and neither is part of Mesa's C target: `mesa_clc` needs
# Clang, LLVM and SPIRV-Tools, none of which Stela's C library carries.
#
# Build them from the same tree the image's Vulkan drivers come from, and put
# them where `cargo xtask gpu nvk` looks for them:
#
#   nix-build --impure -o .tools/gpu/mesa-clc xtask/mesa-clc-native.nix
#
# Rebuild after any change to `third_party/mesa`: the CLC's output is what the
# driver links, and `vtn_bindgen2` writes NIR in the tree's own serialization.
{ pkgs ? import (builtins.getFlake "nixpkgs") { } }:
let
  llvm = pkgs.llvmPackages_21;
  py = pkgs.python3.withPackages (q: [ q.mako q.pyyaml q.packaging q.setuptools ]);
  mesa = ../third_party/mesa;
in
pkgs.stdenv.mkDerivation {
  pname = "stela-mesa-clc-native";
  version = builtins.replaceStrings [ "\n" ] [ "" ] (builtins.readFile (mesa + "/VERSION"));
  src = mesa;
  nativeBuildInputs = [ pkgs.meson pkgs.ninja py pkgs.pkg-config pkgs.bison pkgs.flex ];
  buildInputs = [
    llvm.llvm
    llvm.clang-unwrapped
    pkgs.spirv-tools
    pkgs.spirv-headers
    pkgs.spirv-llvm-translator
  ];
  dontFixup = true;
  dontStrip = true;
  # Mesa looks for one LLVM installation: libLLVM, libclang-cpp and libclang
  # beside each other, with the Clang resource headers under it. Nix splits
  # them, so this gathers them into a prefix and answers llvm-config from it.
  postPatch = ''
    PREFIX=$TMPDIR/llvmprefix
    mkdir -p $PREFIX/lib $PREFIX/clang/21 $TMPDIR/fakebin
    for f in ${llvm.llvm.lib}/lib/libLLVM*.so*; do ln -sf "$f" $PREFIX/lib/; done
    for f in ${llvm.clang-unwrapped.lib}/lib/libclang-cpp*.so*; do ln -sf "$f" $PREFIX/lib/; done
    for f in ${llvm.clang-unwrapped.lib}/lib/libclang.so*; do ln -sf "$f" $PREFIX/lib/; done
    ln -sfn ${llvm.clang-unwrapped.lib}/lib/clang/21/include $PREFIX/clang/21/include
    cat > $TMPDIR/fakebin/llvm-config <<EOF
    #!/bin/sh
    real=${llvm.llvm.dev}/bin/llvm-config
    case "\$1" in
      --libdir) echo $PREFIX/lib; exit 0 ;;
      --cflags) echo "\$(\$real --cflags) -I${llvm.clang-unwrapped}/include"; exit 0 ;;
      --cxxflags) echo "\$(\$real --cxxflags) -I${llvm.clang-unwrapped}/include"; exit 0 ;;
    esac
    exec "\$real" "\$@"
    EOF
    chmod +x $TMPDIR/fakebin/llvm-config
  '';
  configurePhase = ''
    runHook preConfigure
    export PATH=$TMPDIR/fakebin:$PATH
    meson setup build . \
      -Dbuildtype=release -Dvulkan-drivers= -Dgallium-drivers= -Dplatforms= \
      -Dopengl=false -Degl=disabled -Dglx=disabled -Dgles1=disabled -Dgles2=disabled \
      -Dgbm=disabled -Dglvnd=disabled -Dllvm=enabled -Dmesa-clc=enabled -Dspirv-tools=enabled \
      -Dshared-glapi=disabled -Dzstd=disabled -Dzlib=disabled -Dexpat=disabled \
      -Dlibunwind=disabled -Dlmsensors=disabled -Dvalgrind=disabled -Dshader-cache=disabled \
      -Dbuild-tests=false -Dvulkan-layers= -Dinstall-mesa-clc=true -Dgallium-rusticl=false \
      -Dvideo-codecs= -Dselinux=false -Dperfetto=false \
      "-Dc_link_args=-Wl,-rpath,$out/lib" "-Dcpp_link_args=-Wl,-rpath,$out/lib"
    runHook postConfigure
  '';
  buildPhase = ''
    runHook preBuild
    ninja -C build
    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin $out/lib
    cp build/src/compiler/clc/mesa_clc $out/bin/
    find build -name vtn_bindgen2 -type f -exec cp {} $out/bin/ \; 2>/dev/null || true
    for f in ${llvm.llvm.lib}/lib/libLLVM*.so*; do cp -P "$f" $out/lib/; done
    for f in ${llvm.clang-unwrapped.lib}/lib/libclang-cpp*.so*; do cp -P "$f" $out/lib/; done
    for f in ${llvm.clang-unwrapped.lib}/lib/libclang.so*; do cp -P "$f" $out/lib/; done
    for f in ${pkgs.spirv-llvm-translator}/lib/libLLVMSPIRVLib.so*; do cp -P "$f" $out/lib/; done
    for f in ${pkgs.spirv-tools}/lib/libSPIRV-Tools*.so*; do cp -P "$f" $out/lib/; done
    mkdir -p $out/lib/clang/21
    ln -sfn ${llvm.clang-unwrapped.lib}/lib/clang/21/include $out/lib/clang/21/include
    runHook postInstall
  '';
}
