// Rasterize page 1 of a PDF to PNG. Makes a realistic "scan" out of a text PDF.
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let args = Array(CommandLine.arguments.dropFirst())
guard args.count >= 2 else { FileHandle.standardError.write("usage: pdf2png <in.pdf> <out.png> [dpi]\n".data(using: .utf8)!); exit(2) }
let dpi = args.count > 2 ? (Double(args[2]) ?? 200) : 200
guard let doc = CGPDFDocument(URL(fileURLWithPath: args[0]) as CFURL), let page = doc.page(at: 1) else {
    FileHandle.standardError.write("not a pdf\n".data(using: .utf8)!); exit(2)
}
let scale = CGFloat(dpi / 72.0)
let box = page.getBoxRect(.mediaBox)
let w = Int((box.width * scale).rounded()), h = Int((box.height * scale).rounded())
guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                          space: CGColorSpaceCreateDeviceRGB(),
                          bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { exit(2) }
ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
ctx.scaleBy(x: scale, y: scale)
ctx.translateBy(x: -box.origin.x, y: -box.origin.y)
ctx.drawPDFPage(page)
guard let img = ctx.makeImage(),
      let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, UTType.png.identifier as CFString, 1, nil)
else { exit(2) }
CGImageDestinationAddImage(dest, img, nil)
CGImageDestinationFinalize(dest)
print("wrote \(args[1]) \(w)x\(h) @\(Int(dpi))dpi")
