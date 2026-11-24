-- Has a Device Hub window that shows no simulator show this one: select.applescript <UDID>
on run argv
	set udid to item 1 of argv
	tell application "System Events"
		tell process "DeviceHub"
			repeat with w in every window
				if (name of w) starts with "Devices" then
					set theOutline to first UI element of (first scroll area of (first group of (first splitter group of (first group of (first splitter group of (first group of w))))))
					repeat with r in (every row of theOutline)
						try
							if (value of attribute "AXIdentifier" of (first static text of (first UI element of r))) is ("TableRow.Device." & udid) then
								set value of attribute "AXSelected" of r to true
								return "selected in " & (name of w)
							end if
						end try
					end repeat
					return "no row for " & udid
				end if
			end repeat
			return "no empty window"
		end tell
	end tell
end run
