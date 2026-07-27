Installation instructions are short and sweet:

1) Unzip the contents of this file into a folder.
2) Double click on the executable when you want to start the program using your .ini file.

I have included three .ini files. The default NWNArmory.ini file represents the transform
groups from a standard human model (male, female, large and standard phenotypes). If you modify
this file and want it back, copy in standard.ini (they have the same content). Sample.ini
contains some more examples of transforms with comments for when you need to do your own.

For more instructions, double click on the NWNArmory help file (the .chm file) or press
the Help button on the main window.


Enjoy!

Eligio Sacateca
----
<Rumbles Designs>
eligio@cogeco.ca
http://home.cogeco.ca/~eligio/

Change history
=-=-=-=-=-=-=-


*****************
*  Version 1.2  *
*****************

1) Now accepts equivalent functions for transforming texture maps.
2) Fixed a couple of annoying bugs with the file selection dialog (now takes up to 64K in files and should not crash if you cancel out of a selection dialog).
3) Fixed a couple of rare bugs in the transforms for vertices.

*****************
*  Version 1.1  *
*****************

1) Now accepts a "position 0 0 0" line in the source model without complaint. It will apply the standard transforms to this line just like it will to a vertex in the geometry.
2) Now accepts a "position=(x, y, z)" transform to move the position vertex if required. This is an absolute translation (it is not a shift relative to its old position). 
3) Now accepts a "minimum=(x, y, z)" and a "maximum=(x, y, z)" transform. If the vertex being transformed does not fall into this range, it will not be transformed. This may not get used much but it was invaluable to me doing the Drider transforms because the lower half of the torso is scaled more narrowly than the upper half. I ran the torso through NWNArmory several times (still less work than doing 53 female standard + 53 female large + 53 male standard + 53 male large = 212 torsos by hand).
4) Now accepts a "translate=(x,y,z)" transform which will shift all points by the specified values. This may not get used much but was invaluable for the Drider where the pelvis' origin and pivot point was moved relative to the rest of the standard models.

You do not have to change your old .ini files to accommodate these new parameters because they will default if not filled in:
1) The minimum and maximimum parameters default to (-999,-999,-999) and (999,999,999) respectively so they will transform all vertices automatically (again, old behaviour does not change).
2) The translate parameter defaults to (0, 0, 0) so it does not move the vertices (old behaviour).
3) The position parameter defaults to not change the position (old behaviour).