install_ic.sh Script Usage Information
=======================================
1. In Finder, double click on all desired Instant Client .dmg packages to mount them

2. Open a terminal window and change directory to one of the packages, for example:
   $ cd /Volumes/instantclient-basic-macos.arm64-23.3.0.x.x

3. Run the install_ic.sh script:
   $ ./install_ic.sh
   This copies the contents of all currently mounted Instant Client .dmg packages to /Users/$USER/Downloads/instantclient_23_3

4. In Finder, eject the mounted Instant Client packages
