#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint mesh_host.podspec` to validate before publishing.
#
Pod::Spec.new do |s|
  s.name             = 'mesh_host'
  s.version          = '0.1.0'
  s.summary          = 'Mesh Lab native diagnostic host.'
  s.description      = <<-DESC
Mesh Lab native diagnostic host.
                       DESC
  s.homepage         = 'https://github.com/Frazko/Mesh-Lab'
  s.license          = { :file => '../LICENSE' }
  s.author           = 'Mesh Lab'
  s.source           = { :path => '.' }
  s.source_files = 'mesh_host/Sources/mesh_host/**/*'
  s.vendored_frameworks = 'mesh_host/MeshEngine.xcframework'
  s.frameworks = 'CoreBluetooth', 'DeviceDiscoveryUI', 'WiFiAware'
  s.dependency 'Flutter'
  s.platform = :ios, '15.0'

  # Flutter.framework does not contain a i386 slice.
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES', 'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386' }
  s.swift_version = '5.0'

  # If your plugin requires a privacy manifest, for example if it uses any
  # required reason APIs, update the PrivacyInfo.xcprivacy file to describe your
  # plugin's privacy impact, and then uncomment this line. For more information,
  # see https://developer.apple.com/documentation/bundleresources/privacy_manifest_files
  # s.resource_bundles = {'mesh_host_privacy' => ['mesh_host/Sources/mesh_host/PrivacyInfo.xcprivacy']}
end
