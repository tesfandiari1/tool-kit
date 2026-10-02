# Audio fixtures

Generated on this Mac with `say` and `ffmpeg`, so no private recording enters
git. `$T` is any scratch directory, `$F` is `/opt/homebrew/bin/ffmpeg`.

```bash
say -v Samantha -o $T/sam.aiff "The quarterly revenue report shows steady growth across every region we operate in this year."
say -v Daniel -o $T/dan.aiff "I agree with that assessment, although the northern territory still needs a much closer look."
$F -i $T/sam.aiff -f lavfi -t 1 -i anullsrc=r=16000:cl=mono -i $T/dan.aiff -filter_complex "[0:a]aresample=16000,aformat=sample_fmts=s16:channel_layouts=mono[a];[1:a]aformat=sample_fmts=s16:channel_layouts=mono[b];[2:a]aresample=16000,aformat=sample_fmts=s16:channel_layouts=mono[c];[a][b][c]concat=n=3:v=0:a=1" -ar 16000 -ac 1 -c:a pcm_s16le two-speakers.wav
$F -i $T/sam.aiff -t 3 -ar 16000 -ac 1 -c:a aac        -f ipod clip.m4a
$F -i $T/sam.aiff -t 3 -ar 16000 -ac 1 -c:a aac        -f mp4  clip.mp4
$F -i $T/sam.aiff -t 3 -ar 16000 -ac 1 -c:a aac        -f mov  clip.mov
$F -i $T/sam.aiff -t 3 -ar 16000 -ac 1 -c:a libmp3lame -f mp3  clip.mp3
$F -i $T/sam.aiff -t 3 -ar 16000 -ac 1 -c:a flac       -f flac clip.flac
$F -f lavfi -t 2 -i anullsrc=r=16000:cl=mono -c:a pcm_s16le silence.wav
head -c 100 /dev/urandom > garbage.bin
```
