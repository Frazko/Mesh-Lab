# Flutter app

See `../README.md` to prepare the native packages and run the app.

The app has five sections: Network, GPS, Text, Voice, and Diagnostics. In F0,
only bridge verification and state recovery are active. Test fakes live in
`test/`; the app entry point always uses `NativeLabSdk`.

`LabController` is a presentation projection built with ChangeNotifier. It does
not decide connectivity, routes, or delivery. The separation through `LabSdk`
allows Riverpod to be introduced in F5 without changing engine authority or the
plugin contract.
