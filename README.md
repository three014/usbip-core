# usbip-core

A userspace library for interacting with the vhci kernel drivers.

The goal of this library was to integrate usbip functionality with various remote access software such 
as [Moonlight](https://github.com/moonlight-stream/moonlight-qt) and [Sunshine](https://github.com/LizardByte/Sunshine). 

Considering that the real usbip project is baked into the Linux kernel, this project served to help me better
understand how usbip made its connections and hand over control to the kernel drivers.

**Does this project work though?** Yeah, mostly. If you create a rust application using this library, you can attach
and detach devices that other hosts have shared, granted that they're running the official usbip daemon
(this library doesn't have the server-side implemented yet).

Not only that, but this is a rust port of two major libraries, [usbip-win2](https://github.com/vadimgrn/usbip-win2) 
(Windows) and [usbip-utils](https://github.com/torvalds/linux/tree/master/tools/usb/usbip) (Linux). I guess you could
say that my real contribution is creating a cross-platform library, which I think is cool. I would really like to finish 
this project and embed it into a fork of Moonlight/Sunshine one day.
