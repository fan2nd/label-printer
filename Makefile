.PHONY: mac-app
mac-app:
	cargo build --release --locked
	mkdir -p target/release/label-printer.app/Contents/MacOS
	cp assets/Info.plist target/release/label-printer.app/Contents/Info.plist
	install -m 755 target/release/label-printer target/release/label-printer.app/Contents/MacOS/label-printer
	codesign --force --sign - target/release/label-printer.app
