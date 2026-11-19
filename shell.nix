{
  pkgs ? import <nixpkgs> { },
}:

let
  runtimeLibs = [
    pkgs.fontconfig
    pkgs.freetype
    pkgs.libglvnd
    pkgs.libxkbcommon
    pkgs.mesa
    pkgs.vulkan-loader
    pkgs.wayland
  ];
in
pkgs.mkShell {
  nativeBuildInputs = [
    pkgs.pkg-config
  ];

  buildInputs = runtimeLibs;

  shellHook = ''
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath runtimeLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

    if [ -z "''${VK_DRIVER_FILES:-}" ] && [ -z "''${VK_ICD_FILENAMES:-}" ] && [ -d "${pkgs.mesa}/share/vulkan/icd.d" ]; then
      _zgui_icds=""
      for _zgui_icd in \
        "${pkgs.mesa}/share/vulkan/icd.d"/radeon_icd.*.json \
        "${pkgs.mesa}/share/vulkan/icd.d"/intel_icd.*.json \
        "${pkgs.mesa}/share/vulkan/icd.d"/intel_hasvk_icd.*.json \
        "${pkgs.mesa}/share/vulkan/icd.d"/nouveau_icd.*.json \
        "${pkgs.mesa}/share/vulkan/icd.d"/virtio_icd.*.json \
        "${pkgs.mesa}/share/vulkan/icd.d"/lvp_icd.*.json; do
        [ -e "$_zgui_icd" ] || continue
        _zgui_icds="''${_zgui_icds:+$_zgui_icds:}$_zgui_icd"
      done
      if [ -z "$_zgui_icds" ]; then
        for _zgui_icd in "${pkgs.mesa}/share/vulkan/icd.d"/*.json; do
          [ -e "$_zgui_icd" ] || continue
          _zgui_icds="''${_zgui_icds:+$_zgui_icds:}$_zgui_icd"
        done
      fi
      if [ -n "$_zgui_icds" ]; then
        export VK_DRIVER_FILES="$_zgui_icds"
      fi
      unset _zgui_icds _zgui_icd
    fi
  '';
}
