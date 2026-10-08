//! Native Vision text recognition. No compiler or UI process is started for OCR.
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

#[cfg(target_os = "macos")]
pub fn read(path: &Path) -> Result<Value> {
    use anyhow::{Context, anyhow};
    use objc2::{AnyThread, rc::autoreleasepool};
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSArray, NSDictionary, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };
    use serde_json::json;
    autoreleasepool(|_| {
        let filename = NSString::from_str(path.to_str().context("OCR image path must be UTF-8")?);
        let source = NSImage::initWithContentsOfFile(NSImage::alloc(), &filename)
            .context("Cannot open OCR image")?;
        // SAFETY: the optional destination pointer is null and no context/hints are supplied.
        // The returned CGImage and its source stay retained through the synchronous request.
        let picture = unsafe {
            source.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None)
        }
        .context("Cannot decode OCR image")?;
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(false);
        // SAFETY: the image is retained and immutable, and the correctly typed options dictionary is empty.
        let handler = unsafe {
            VNImageRequestHandler::initWithCGImage_options(
                VNImageRequestHandler::alloc(),
                &picture,
                &NSDictionary::new(),
            )
        };
        let requests = NSArray::<VNRequest>::from_slice(&[&request]);
        handler
            .performRequests_error(&requests)
            .map_err(|error| anyhow!("Vision OCR unavailable: {error}"))?;
        let mut rows = Vec::new();
        if let Some(results) = request.results() {
            for index in 0..results.count() {
                let observation = results.objectAtIndex(index);
                let candidates = observation.topCandidates(1);
                if let Some(text) = candidates.firstObject() {
                    // SAFETY: this initialized, retained text observation was produced by Vision.
                    let bounds = unsafe { observation.boundingBox() };
                    rows.push(json!({"text":text.string().to_string(),
                        "x":bounds.origin.x+ bounds.size.width/2.,
                        "y":1.-bounds.origin.y-bounds.size.height/2.}));
                }
            }
        }
        Ok(json!(rows))
    })
}

#[cfg(not(target_os = "macos"))]
pub fn read(_path: &Path) -> Result<Value> {
    anyhow::bail!("Native screen OCR requires macOS Vision")
}
