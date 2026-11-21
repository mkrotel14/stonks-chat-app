# Package the exact release executable built and tested by Fedora CI.
%global debug_package %{nil}
%global __spec_install_post %{nil}
%global _build_id_links none

Name:           stonks-gpui
Version:        %{stonks_version}
Release:        1%{?dist}
Summary:        Stonks Chat native GPUI client for Stoat
License:        Apache-2.0
URL:            https://github.com/mkrotel14/stonks-chat-app
ExclusiveArch:  x86_64
Source0:        stonks-app
Source1:        com.rdxlabs.stonks.gpui.desktop
Source2:        com.rdxlabs.stonks.gpui.png
Source3:        LICENSE
Source4:        THIRD_PARTY_NOTICES.md

# RPM generates ELF requirements. These cover plugins, dlopen and tools.
Requires:       gstreamer1%{?_isa} >= 1.28
Requires:       gstreamer1-plugins-base%{?_isa} >= 1.28
Requires:       gstreamer1-plugins-good%{?_isa} >= 1.28
Requires:       gstreamer1-plugins-bad-free%{?_isa} >= 1.28
Requires:       pipewire-gstreamer%{?_isa}
Requires:       pipewire-utils
Requires:       vulkan-loader%{?_isa}
Requires:       libxkbcommon-x11%{?_isa}
Requires:       fontconfig%{?_isa}
Requires:       dejavu-sans-fonts
Requires:       hicolor-icon-theme
Requires:       libdrm%{?_isa}, mesa-libgbm%{?_isa}
Requires:       libX11%{?_isa}, libXfixes%{?_isa}, libXdamage%{?_isa}
Requires:       libXrandr%{?_isa}, libXcomposite%{?_isa}, libXext%{?_isa}
Recommends:     mesa-vulkan-drivers%{?_isa}
Recommends:     pipewire-pulseaudio
Suggests:       ffmpeg-free, yt-dlp, deno

%description
Native GPUI desktop client for Stoat, with chat, media playback and voice.

%install
install -Dm755 %{SOURCE0} %{buildroot}%{_bindir}/stonks-app
install -Dm644 %{SOURCE1} %{buildroot}%{_datadir}/applications/com.rdxlabs.stonks.gpui.desktop
install -Dm644 %{SOURCE2} %{buildroot}%{_datadir}/icons/hicolor/128x128/apps/com.rdxlabs.stonks.gpui.png
install -Dm644 %{SOURCE3} %{buildroot}%{_licensedir}/%{name}/LICENSE
install -Dm644 %{SOURCE4} %{buildroot}%{_licensedir}/%{name}/THIRD_PARTY_NOTICES.md

%files
%{_bindir}/stonks-app
%{_datadir}/applications/com.rdxlabs.stonks.gpui.desktop
%{_datadir}/icons/hicolor/128x128/apps/com.rdxlabs.stonks.gpui.png
%dir %{_licensedir}/%{name}
%license %{_licensedir}/%{name}/LICENSE
%license %{_licensedir}/%{name}/THIRD_PARTY_NOTICES.md
