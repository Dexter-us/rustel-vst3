; FM Bass
; Rustel csoundm events provide p4 as a fractional MIDI note and p5 as
; velocity-scaled level in the 0..127 range.
; Rustel supplies stereo output and a 0dbfs reference of 1.

instr fm_bass
  iMidi limit p4, 28, 67
  iRawVelocity = p5 / 127
  iVelocity limit iRawVelocity, 0, 1
  iFrequency = cpsmidinn(iMidi)
  iLevel = iVelocity * 0.095

  aEnvelope madsr 0.006, 0.12, 0.72, 0.08
  aIndex madsr 0.004, 0.14, 0.10, 0.06
  aModulator oscili iFrequency * (1.35 * aIndex), iFrequency * 2
  aBody oscili iLevel * aEnvelope, iFrequency + aModulator
  aSub oscili iLevel * 0.20 * aEnvelope, iFrequency * 0.5
  aOutput = (aBody + aSub) * 0.82
  outs aOutput, aOutput
endin
