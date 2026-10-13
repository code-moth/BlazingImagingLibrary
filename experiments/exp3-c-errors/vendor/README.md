# Headers for experiment 3

The C layer of experiment 3 is compiled against these headers, and linked against the libraries the system provides. They are here because the workspace the experiment was written in had the libraries' runtimes but not their development packages. They are used only by the experiment and never reach a BIL package.

| Folder | From | License |
| --- | --- | --- |
| `libjpeg-turbo-2.1.5/` | `jpeglib.h`, `jmorecfg.h` and `jerror.h` of the libjpeg-turbo 2.1.5 release, unchanged | The IJG License and the Modified (3-clause) BSD License, in `LICENSE.md` and `README.ijg` |
| `openjpeg-2.5.0/` | `openjpeg.h` of the OpenJPEG 2.5.0 release, unchanged | The 2-clause BSD License, in `LICENSE` |

Three files are written for the experiment, in place of files each release generates when it is built: `jconfig.h`, from libjpeg-turbo's `jconfig.h.in`, with the settings of Ubuntu 24.04's `libjpeg-turbo8` (the libjpeg 8 interface, 8-bit samples, memory sources and destinations); and `opj_config.h` and `opj_stdint.h`, from OpenJPEG's templates. Each says so at its top.

**How a mismatch would show.** libjpeg checks the size of the structures it is handed against its own, and reports a difference as an error, so headers that did not match the library would fail every call rather than corrupt memory. OpenJPEG has no such check: its handles are opaque, but its image and parameter structures are read and written directly, so the header must be the release linked. Ubuntu 24.04 ships OpenJPEG 2.5.0, and the experiment's first check, that its seed files come back losslessly, would fail if the layouts differed.
