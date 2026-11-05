# What a window playing video costs

webviewrs sets no media or GPU options, and its own process stays near 230 MB. The cost of a video
page sits in `WebKitWebProcess`, so the system's WebKitGTK, GStreamer and Mesa decide it.

These figures come from a YouTube playlist in a 1906x1046 window, on WebKitGTK 2.54.2, GStreamer
1.28.8 and Mesa 26.2.6, with an AMD Radeon 680M driving the display. CPU is percent of one core,
for the whole web process.

| | Software decode | VA-API decode on the display's GPU |
| --- | --- | --- |
| Stream YouTube chose | VP9 in Matroska | AV1 in MP4 |
| Decoder | `vp9dec` | `varenderD129av1dec` |
| Web process CPU | about 56% | about 28% |
| Compositor thread | about 20% | about 19% |
| Decode threads | about 19% | none above 0.5% |
| GTT memory | about 0.73 GB | about 0.69 GB |
| RSS range after 4 minutes | 1.18 to 1.57 GB | 1.07 to 1.64 GB |

## Hardware decoding needs a plugin WebKitGTK does not pull in

On Arch, `gst-plugin-va` is a separate package. Without it, WebKit decodes on the CPU and prints
nothing about it. With it, the same playlist played AV1 instead of VP9.

On a machine with two GPUs, GStreamer gives the unnamed `va*` decoders to the first render node.
Each other node gets decoders named after it, such as `varenderD129av1dec`, at a lower rank. So the
default can decode on a GPU that is not driving the display. `GST_PLUGIN_FEATURE_RANK` moves it:

```sh
GST_PLUGIN_FEATURE_RANK=vaav1dec:NONE,varenderD129av1dec:MAX
```

`gst-inspect-1.0 va` lists the decoders and the GPU behind each.

## Memory climbs and drops back, and the decoder does not change it

RSS climbs by about 2.7 MB a second, almost all of it in WebKit's `fastmalloc` heap. It drops by
0.3 to 0.6 GB when it reaches about 1.6 GB. Software and hardware decoding show the same pattern. A
window left open for 88 minutes was at 1.64 GB of RSS, so this is a high plateau rather than a leak.

What fills the heap is not established.

## One crash is on record, and no cause is established

The web process died once with SIGSEGV on its compositor thread, in Mesa's
`_mesa_release_pending_resource` (`mesa/main/shared.c:510`), during `glDeleteBuffers` through
glthread. A short run under an older Mesa cannot test this, because the crash takes far longer to
appear than any such run lasts. A second core dump with the same stack would make it reportable
upstream.
