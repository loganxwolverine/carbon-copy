# Paquet binaire : le programme est compilé par packaging/build.sh puis
# copié tel quel (variable « stage »). Les dépendances vers GTK et
# libadwaita sont calculées automatiquement par rpmbuild.
Name:           intact-bitstream
Version:        %{ver}
Release:        %{rel}
Summary:        Vos DVD et Blu-ray en MKV, sans perte (interface GTK4 pour MakeMKV)
License:        GPL-3.0-or-later
URL:            https://github.com/loganxwolverine/intact-bitstream
Requires:       flatpak
Requires:       mkvtoolnix
Requires:       hicolor-icon-theme

%description
Intact Bitstream pilote MakeMKV pour extraire vos DVD et Blu-ray en fichiers MKV :
reconnaissance du film avec son affiche, choix automatique des pistes, nommage
complet du fichier et éjection en fin de copie. MakeMKV est installé et mis à
jour automatiquement depuis Flathub.

%global debug_package %{nil}
%global __brp_strip %{nil}

%install
cp -a %{stage}/. %{buildroot}/

%post
modprobe sg >/dev/null 2>&1 || :
udevadm control --reload-rules >/dev/null 2>&1 || :
udevadm trigger --subsystem-match=scsi_generic >/dev/null 2>&1 || :

%files
%license /usr/share/licenses/intact-bitstream/LICENSE
/usr/bin/intact-bitstream
/usr/share/applications/io.github.loganxwolverine.IntactBitstream.desktop
/usr/share/metainfo/io.github.loganxwolverine.IntactBitstream.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/io.github.loganxwolverine.IntactBitstream.svg
/usr/lib/udev/rules.d/70-intact-bitstream.rules
/usr/lib/modules-load.d/intact-bitstream.conf
%dir /usr/share/licenses/intact-bitstream
