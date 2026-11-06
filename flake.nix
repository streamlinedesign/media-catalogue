{
  description = "Katalog: a personal media library. Music first.";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = forAll (pkgs: rec {
        katalog = pkgs.rustPlatform.buildRustPackage {
          pname = "katalog";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
          buildInputs = [ pkgs.alsa-lib ];
          # The window loads these at run time (winit and wgpu dlopen them).
          postFixup = ''
            wrapProgram $out/bin/katalog-window \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (with pkgs; [
                libxkbcommon wayland xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr
                vulkan-loader libGL
              ])}
          '';
          meta.mainProgram = "katalogd";
        };
        default = katalog;
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc rust-analyzer clippy rustfmt pkg-config alsa-lib libxkbcommon wayland xorg.libX11 vulkan-loader libGL ffmpeg ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [ libxkbcommon wayland xorg.libX11 vulkan-loader libGL ]);
        };
      });

      # Run the daemon as a user service: `systemctl --user enable --now katalogd`.
      # Put this in your home-manager or NixOS config:
      #   systemd.user.services.katalogd = katalog.lib.userService pkgs;
      lib.userService = pkgs: {
        Unit.Description = "Katalog music library daemon";
        Service = {
          ExecStart = "${self.packages.${pkgs.system}.katalog}/bin/katalogd";
          Restart = "on-failure";
        };
        Install.WantedBy = [ "default.target" ];
      };
    };
}
