with open("web/src/components/GlobalProgressBanners.tsx", "r") as f:
    content = f.read()

# Completely rewrite the minimum-display and rendering logic to be bulletproof.
replacement = """function useMinimumDisplay(
  active: boolean,
  minMs: number = 8000,
): [boolean, boolean, () => void] {
  const [visible, setVisible] = useState(false);
  const showSinceRef = useRef<number | null>(null);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const forceHide = () => {
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    hideTimerRef.current = null;
    showSinceRef.current = null;
    setVisible(false);
  };

  useEffect(() => {
    if (active) {
      if (hideTimerRef.current) {
        clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
      }
      if (!showSinceRef.current) showSinceRef.current = Date.now();
      setVisible(true);
    } else if (visible && showSinceRef.current) {
      const elapsed = Date.now() - showSinceRef.current;
      const remaining = Math.max(0, minMs - elapsed);
      if (remaining === 0) {
        forceHide();
      } else {
        hideTimerRef.current = setTimeout(forceHide, remaining);
      }
    }
    return () => {
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
    };
  }, [active]);

  return [visible, visible && !active, forceHide];
}"""

# Extract everything from useMinimumDisplay up to the end of the file and manually patch it.
# Actually let's use regex to replace useMinimumDisplay
import re

content = re.sub(r'function useMinimumDisplay.*?return \[visible, visible && !active\];\n}', replacement, content, flags=re.DOTALL)

# Now fix the variables in GlobalProgressBanners
old_vars = """  // Apply minimum-display behavior
  const [showConversion, conversionDone] = useMinimumDisplay(conversionBusy);
  
  // If migration activates, force the conversion banner to close so it doesn't queue a "complete" state.
  const bypassConversionDisplay = conversionSuppressed || (!showConversion && conversionPending === 0 && conversionMissingThumbs === 0);

  const [showMigration, migrationDone] = useMinimumDisplay(migrationBusy);"""

new_vars = """  // Apply minimum-display behavior
  const [showConversion, conversionDone, forceHideConversion] = useMinimumDisplay(conversionBusy);
  const [showMigration, migrationDone] = useMinimumDisplay(migrationBusy);

  // If a migration is forcefully suppressing conversion, immediately cancel its display state
  // to avoid a "phantom completion flash" when migration finishes.
  useEffect(() => {
    if (conversionSuppressed) {
      forceHideConversion();
    }
  }, [conversionSuppressed, forceHideConversion]);
"""
content = content.replace(old_vars, new_vars)

# Fix the render check
old_render = """  if ((bypassConversionDisplay || !showConversion) && !showMigration) return null;"""
new_render = """  if (!showConversion && !showMigration) return null;"""
content = content.replace(old_render, new_render)

# Remove the !bypassConversionDisplay from JSX
old_jsx = """      {showConversion && !bypassConversionDisplay && ("""
new_jsx = """      {showConversion && ("""
content = content.replace(old_jsx, new_jsx)

# Also fix the decryption typo fallback
old_mig = """                {migrationStatus === "decrypting" ? "Decryption" : "Encryption"} complete
              </p>"""
new_mig = """                {encryptionMode === "plain" ? "Decryption" : "Encryption"} complete
              </p>"""
content = content.replace(old_mig, new_mig)


with open("web/src/components/GlobalProgressBanners.tsx", "w") as f:
    f.write(content)

