; FM Bell
; Rustel csoundm events provide p4 as a fractional MIDI note and p5 as
; velocity-scaled level in the 0..127 range.
; Rustel supplies stereo output and a 0dbfs reference of 1.

instr fm_bell
  iMidi limit p4, 48, 108
  iRawVelocity = p5 / 127
  iVelocity limit iRawVelocity, 0, 1
  iFrequency = cpsmidinn(iMidi)
  iLevel = iVelocity * 0.075

  aEnvelope madsr 0.002, 1.25, 0.03, 0.35
  aIndex madsr 0.001, 0.90, 0.02, 0.28
  aModulator oscili iFrequency * (4.2 * aIndex), iFrequency * 2.73
  aStrike oscili iLevel * aEnvelope, iFrequency + aModulator
  aPartial oscili iLevel * 0.18 * aEnvelope, iFrequency * 2.4 + aModulator * 0.31
  aOutput = (aStrike + aPartial) * 0.80
  outs aOutput, aOutput
endin
