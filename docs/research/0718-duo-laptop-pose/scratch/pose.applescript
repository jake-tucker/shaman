-- Presses one of the buttons under an iPhone Duo simulator in Device Hub, without bringing Device Hub forward.
--
--   osascript pose.applescript <UDID> <simulator name> "Open"
--
-- The button is named as its tooltip names it: "Closed", "Partially Open", "Open", "Rotate Right", "Home",
-- "Screenshot" or "Record Screen". With no button named, the buttons are listed.
--
-- Device Hub must be running, and one of its windows must show this simulator on the Space in front: a window on
-- another Space is not among the windows accessibility answers with, and this says so and presses nothing. A window is
-- found by its title, "<simulator name> – iOS <version>", never by its place in the list: the window in front may be
-- another checkout's simulator. Where no window shows the simulator, a new one is opened and the simulator chosen in
-- its sidebar by UDID.
on run argv
	set udid to item 1 of argv
	set deviceName to item 2 of argv
	set wanted to ""
	if (count of argv) > 2 then set wanted to item 3 of argv
	tell application "System Events"
		if not (exists process "DeviceHub") then error "Device Hub is not running: open -g -a /Applications/Xcode.app/Contents/Applications/DeviceHub.app"
		tell process "DeviceHub"
			set found to my windowFor(deviceName)
			if found is missing value then
				click menu item "New Window" of menu 1 of menu bar item "File" of menu bar 1
				delay 2
				-- The new window is in front, and shows whichever simulator Device Hub chose.
				set fresh to window 1
				set theOutline to first UI element of (first scroll area of (first group of (first splitter group of (first group of (first splitter group of (first group of fresh))))))
				repeat with r in (every row of theOutline)
					try
						if (value of attribute "AXIdentifier" of (first static text of (first UI element of r))) is ("TableRow.Device." & udid) then
							set value of attribute "AXSelected" of r to true
							exit repeat
						end if
					end try
				end repeat
				delay 3
				set found to my windowFor(deviceName)
				if found is missing value then error "No Device Hub window shows " & deviceName
			end if
			-- Several windows may show this simulator, and one newly made answers with no tooltips yet: press in the first
			-- that has the button.
			set listing to ""
			repeat with w in every window
				if (name of w) starts with (deviceName & " – ") then
					try
						set detail to group 3 of (first splitter group of (first group of (first splitter group of (first group of w))))
						repeat with b in (every button of detail)
							set tip to ""
							try
								set tip to value of attribute "AXHelp" of b
							end try
							if wanted is not "" and tip is wanted then
								perform action "AXPress" of b
								return "pressed " & wanted
							end if
							set listing to listing & tip & linefeed
						end repeat
					end try
					set listing to listing & "--" & linefeed
				end if
			end repeat
			if wanted is "" then return listing
			error "No button named " & wanted & " among: " & listing
		end tell
	end tell
end run

on windowFor(deviceName)
	tell application "System Events"
		tell process "DeviceHub"
			repeat with w in every window
				if (name of w) starts with (deviceName & " – ") then return w
			end repeat
		end tell
	end tell
	return missing value
end windowFor
