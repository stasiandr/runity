-- The Scrap launcher (`tools/studio-app.sh --launcher` compiles it into
-- ~/Applications/Scrap.app). hop, Dock and Finder open it instead of the
-- editor: it opens the editor built from what is on disk now, and when that
-- takes a build, shows the build's progress in a window — cargo's own count
-- of crates, what it is compiling, how long it has been.
--
-- The work is tools/studio-open.sh's; REPO is put in at compile time.

property repo : "REPO"

on tool(step)
	return do shell script quoted form of (repo & "/tools/studio-open.sh") & " " & step
end tool

on run
	if tool("start") is "running" then
		tool("open")
		return
	end if
	set shown to false
	try
		repeat
			set AppleScript's text item delimiters to "|"
			set parts to text items of tool("poll")
			set AppleScript's text item delimiters to ""
			set phase to item 1 of parts
			if phase is "done" then exit repeat
			if phase is "building" then
				if not shown then
					set progress description to "Код изменился — собираю свежий Scrap"
					-- Opened from hop the launcher is not always in front.
					activate
					set shown to true
				end if
				set units to (item 3 of parts) as integer
				if units > 0 then
					set progress total steps to units
					set progress completed steps to (item 2 of parts) as integer
					set progress additional description to (item 5 of parts) & " · " & (item 2 of parts) & "/" & units & " · " & (item 4 of parts)
				else
					set progress total steps to -1
					set progress additional description to (item 5 of parts)
				end if
			end if
			delay 0.5
		end repeat
	on error number -128
		-- Stop in the progress window.
		tool("stop")
		return
	end try

	if (item 2 of parts) is "0" then
		tool("open")
		return
	end if
	set answer to button returned of (display dialog "Scrap не собрался." buttons {"Закрыть", "Лог", "Открыть прежнюю"} default button 3 cancel button 1 with title "Scrap" with icon caution)
	if answer is "Лог" then
		tool("log")
	else
		tool("open")
	end if
end run
