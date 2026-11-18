; FM Electric Piano
; Rustel csoundm events provide p4 as a fractional MIDI note and p5 as
; velocity-scaled level in the 0..127 range.
; Rustel supplies stereo output and a 0dbfs reference of 1.

instr fm_electric_piano
  iMidi limit p4, 24, 100
  iRawVelocity = p5 / 127
  iVelocity limit iRawVelocity, 0, 1
  iFrequency = cpsmidinn(iMidi)
  iLevel = iVelocity * 0.10

  aEnvelope madsr 0.004, 0.24, 0.16, 0.18
  aIndex madsr 0.002, 0.32, 0.06, 0.14
  aModulator oscili iFrequency * (2.6 * aIndex), iFrequency
  aFundamental oscili iLevel * aEnvelope, iFrequency + aModulator
  aTine oscili iLevel * 0.18 * aEnvelope, iFrequency * 2
  aOutput = (aFundamental + aTine) * 0.82
  outs aOutput, aOutput
endin
